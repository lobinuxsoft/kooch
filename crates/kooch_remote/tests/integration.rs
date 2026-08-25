//! End-to-end tests: `handle` against a live ECS, and a full HTTP
//! round-trip through the listener thread and main-loop bridge.

use std::io::{BufRead, BufReader, Write};

use interprocess::local_socket::traits::Stream as _;
use interprocess::local_socket::{GenericNamespaced, Stream, ToNsName};

use kooch_core::resource::Resources;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::commands::Commands;
use kooch_ecs::component::{ComponentNames, ComponentRegistry};
use kooch_ecs::dynamic_components::DynamicComponents;
use kooch_ecs::name::Name;
use kooch_ecs::query::AccessTracker;
use kooch_ecs::reflect::ReflectValue;
use kooch_ecs::transform::Transform;

use kooch_remote::client::RemoteClient;
use kooch_remote::handlers::handle;
use kooch_remote::protocol::{Method, Request, ResponseData, ResponsePayload};
use kooch_remote::server::RemoteServer;

/// A minimal ECS with the resources the handlers touch, plus `Name` and
/// `Transform` registered as reflected components.
fn ecs() -> Resources {
    let mut resources = Resources::new();
    resources.insert(EntityAllocator::new());
    resources.insert(ComponentRegistry::new());
    resources.insert(ArchetypeRegistry::new());
    resources.insert(AccessTracker::new());
    resources.insert(Commands::new());
    resources.insert(DynamicComponents::new());
    resources.insert(ComponentNames::new());
    let registry = resources.get_mut::<ComponentRegistry>().unwrap();
    registry.register_cpu_reflected::<Name>();
    registry.register_cpu_reflected::<Transform>();
    // The hierarchy and ordering types a real host gets from `EcsPlugin`.
    // Without them `reparent` and `place` find no storage and do nothing
    // — silently, which is how a fixture ends up testing the absence of
    // a feature rather than the feature.
    registry.register_cpu_reflected::<kooch_ecs::hierarchy::Parent>();
    registry.register_cpu_reflected::<kooch_ecs::hierarchy::Children>();
    registry.register_cpu_reflected::<kooch_ecs::Order>();
    resources
}

fn call(resources: &mut Resources, method: Method) -> ResponseData {
    let response = handle(&Request { id: 1, method }, resources);
    match response.payload {
        ResponsePayload::Result(data) => data,
        ResponsePayload::Error(e) => panic!("unexpected error: {e:?}"),
    }
}

#[test]
fn spawn_set_field_and_list_round_trip() {
    let mut resources = ecs();

    // Spawn a named entity.
    let entity = match call(
        &mut resources,
        Method::Spawn {
            name: Some("Hero".into()),
            scene: None,
            parent: None,
        },
    ) {
        ResponseData::Spawned { entity } => entity,
        other => panic!("expected Spawned, got {other:?}"),
    };

    // It shows up in the listing with its name.
    let listed = match call(&mut resources, Method::ListEntities { since: None }) {
        ResponseData::Entities { entities, .. } => entities,
        other => panic!("expected Entities, got {other:?}"),
    };
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, entity);
    assert_eq!(listed[0].name.as_deref(), Some("Hero"));

    // Add a Transform, then set a field on it through the protocol.
    call(
        &mut resources,
        Method::AddComponent {
            entity,
            component: std::any::type_name::<Transform>().into(),
        },
    );
    call(
        &mut resources,
        Method::SetField {
            entity,
            component: std::any::type_name::<Transform>().into(),
            field: "position".into(),
            value: ReflectValue::Vec3(glam::Vec3::new(1.0, 2.0, 3.0)),
        },
    );

    // The listing reflects the edit.
    let listed = match call(&mut resources, Method::ListEntities { since: None }) {
        ResponseData::Entities { entities, .. } => entities,
        other => panic!("expected Entities, got {other:?}"),
    };
    let transform = listed[0]
        .components
        .iter()
        .find(|c| c.type_name.ends_with("Transform"))
        .expect("Transform present");
    let position = transform
        .fields
        .iter()
        .find(|(n, _)| n == "position")
        .map(|(_, v)| v);
    assert_eq!(
        position,
        Some(&ReflectValue::Vec3(glam::Vec3::new(1.0, 2.0, 3.0)))
    );
}

/// A spawned entity carries `Name` and `Transform` whether or not a name
/// came with it.
///
/// "Spawn → Entity" in the World panel sends no name, and this path used
/// to add `Name` only when one was given — so a remote project produced an
/// entity the Inspector could not rename, because the name editor reads
/// the component and there was none. The editor's local spawn has always
/// added both; the two paths have to agree or the same menu entry means
/// two different things.
#[test]
fn a_nameless_spawn_still_carries_name_and_transform() {
    let mut resources = ecs();
    let entity = match call(
        &mut resources,
        Method::Spawn {
            name: None,
            scene: None,
            parent: None,
        },
    ) {
        ResponseData::Spawned { entity } => entity,
        other => panic!("{other:?}"),
    };

    let entities = match call(&mut resources, Method::ListEntities { since: None }) {
        ResponseData::Entities { entities, .. } => entities,
        other => panic!("{other:?}"),
    };
    let spawned = entities
        .iter()
        .find(|e| e.id == entity)
        .expect("the spawned entity is listed");
    let carried: Vec<&str> = spawned
        .components
        .iter()
        .map(|c| c.type_name.rsplit("::").next().unwrap_or(&c.type_name))
        .collect();

    assert!(carried.contains(&"Name"), "no Name component: {carried:?}");
    assert!(
        carried.contains(&"Transform"),
        "no Transform component: {carried:?}",
    );
}

#[test]
fn unknown_component_is_a_typed_error() {
    let mut resources = ecs();
    let entity = match call(
        &mut resources,
        Method::Spawn {
            name: None,
            scene: None,
            parent: None,
        },
    ) {
        ResponseData::Spawned { entity } => entity,
        other => panic!("{other:?}"),
    };
    let response = handle(
        &Request {
            id: 2,
            method: Method::AddComponent {
                entity,
                component: "game::NotHere".into(),
            },
        },
        &mut resources,
    );
    match response.payload {
        ResponsePayload::Error(kooch_remote::protocol::RemoteError::UnknownComponent {
            type_name,
        }) => {
            assert_eq!(type_name, "game::NotHere");
        }
        other => panic!("expected UnknownComponent, got {other:?}"),
    }
}

/// A socket name unique to this test.
///
/// Tests run in parallel in one process, so a shared name would have them
/// binding over each other — the local-socket equivalent of the port
/// scan this replaced, but solved instead of retried.
fn test_socket_name() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static N: AtomicU32 = AtomicU32::new(0);
    // The counter alone is not enough: it is per-module, so two test
    // modules in one binary both start at zero and collide on the same
    // name. The clock disambiguates without the modules having to know
    // about each other.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    format!(
        "kooch_test_{}_{}_{}.sock",
        std::process::id(),
        nanos,
        N.fetch_add(1, Ordering::Relaxed)
    )
}

/// The wire format, exercised without `RemoteClient`.
///
/// Deliberately raw: if this only went through the client, the two could
/// drift into agreeing on something the protocol does not actually
/// specify. One JSON object per line, in and out.
#[test]
fn a_raw_line_round_trips_through_the_bridge() {
    let server = RemoteServer::start(&test_socket_name()).expect("bind a socket");
    let socket = server.name().to_owned();

    // Client thread: write one line, read one line.
    let client = std::thread::spawn(move || {
        let name = socket
            .as_str()
            .to_ns_name::<GenericNamespaced>()
            .expect("valid name");
        let stream = Stream::connect(name).expect("connect");
        let mut conn = BufReader::new(stream);
        conn.get_mut()
            .write_all(b"{\"id\":42,\"method\":\"ping\"}\n")
            .unwrap();
        let mut response = String::new();
        conn.read_line(&mut response).unwrap();
        response
    });

    // "Main loop": drain the queue and answer, bounded so a failure ends
    // the test rather than hanging it.
    let mut resources = ecs();
    let mut answered = false;
    for _ in 0..2000 {
        for item in server.take_pending() {
            let response = handle(&item.request, &mut resources);
            item.reply.send(response).unwrap();
            answered = true;
        }
        if answered {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(answered, "server never received the request");

    let response = client.join().unwrap();
    assert!(response.contains("\"kind\":\"pong\""), "body: {response}");
    assert!(response.contains("\"id\":42"), "id echoed: {response}");
    assert!(
        response.ends_with('\n'),
        "replies are line-delimited: {response:?}"
    );
}

#[test]
fn client_drives_server_end_to_end() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let server = RemoteServer::start(&test_socket_name()).expect("bind a port");
    let socket = server.name().to_owned();

    // The project's "main loop": owns the ECS, drains the queue and
    // answers, until the client signals it is done.
    let done = Arc::new(AtomicBool::new(false));
    let loop_done = Arc::clone(&done);
    let main_loop = std::thread::spawn(move || {
        let mut resources = ecs();
        while !loop_done.load(Ordering::Relaxed) {
            for item in server.take_pending() {
                let response = handle(&item.request, &mut resources);
                let _ = item.reply.send(response);
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    });

    // The editor client: typed calls over the wire.
    let client = RemoteClient::new(&socket);
    client.ping().expect("ping");

    let hero = client.spawn(Some("Hero"), None, None).expect("spawn");
    client
        .add_component(hero, std::any::type_name::<Transform>())
        .expect("add component");
    client
        .set_field(
            hero,
            std::any::type_name::<Transform>(),
            "position",
            ReflectValue::Vec3(glam::Vec3::new(4.0, 5.0, 6.0)),
        )
        .expect("set field");

    let entities = client.list_entities().expect("list");
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].id, hero);
    assert_eq!(entities[0].name.as_deref(), Some("Hero"));
    let position = entities[0]
        .components
        .iter()
        .find(|c| c.type_name.ends_with("Transform"))
        .and_then(|c| c.fields.iter().find(|(n, _)| n == "position"))
        .map(|(_, v)| v);
    assert_eq!(
        position,
        Some(&ReflectValue::Vec3(glam::Vec3::new(4.0, 5.0, 6.0)))
    );

    // #645 — every call records what it cost, split into the wait for
    // the server's main thread and the parse of what came back. The
    // editor pulls this every frame while playing, so it has to be
    // possible to tell which half is the bill.
    let stats = client.last_call_stats();
    assert!(
        stats.response_bytes > 0,
        "a listing that returned an entity cannot be zero bytes"
    );
    assert!(
        stats.transport_us > 0,
        "the main loop sleeps a millisecond per turn; the wait is measurable"
    );

    // A schema call reaches the registry.
    let schema = client.get_schema().expect("schema");
    assert!(schema.iter().any(|c| c.type_name.ends_with("Transform")));

    // And the sample is per call, not cumulative: the schema call above
    // replaced the listing's numbers rather than adding to them.
    let after_schema = client.last_call_stats();
    assert_ne!(
        after_schema.response_bytes, stats.response_bytes,
        "the stats did not move to the newer call"
    );

    // An unknown component surfaces as a typed remote error, not a hang.
    let err = client
        .add_component(hero, "game::NotHere")
        .expect_err("unknown component must error");
    assert!(
        matches!(
            err,
            kooch_remote::ClientError::Remote(
                kooch_remote::protocol::RemoteError::UnknownComponent { .. }
            )
        ),
        "got {err:?}"
    );

    done.store(true, Ordering::Relaxed);
    main_loop.join().unwrap();
}

/// Entities are listed in the order the user authored them, not grouped
/// by archetype — a hierarchy panel shows this order verbatim.
#[test]
fn entities_are_listed_in_authored_order() {
    let mut resources = ecs();
    let transform_ty = std::any::type_name::<Transform>().to_owned();

    // Give the second entity a component the others lack, so archetype
    // grouping would pull it out of order if the listing relied on it.
    for (i, name) in ["First", "Second", "Third"].iter().enumerate() {
        let entity = match call(
            &mut resources,
            Method::Spawn {
                name: Some((*name).into()),
                scene: None,
                parent: None,
            },
        ) {
            ResponseData::Spawned { entity } => entity,
            other => panic!("spawn: {other:?}"),
        };
        if i == 1 {
            call(
                &mut resources,
                Method::AddComponent {
                    entity,
                    component: transform_ty.clone(),
                },
            );
        }
    }

    let entities = match call(&mut resources, Method::ListEntities { since: None }) {
        ResponseData::Entities { entities, .. } => entities,
        other => panic!("list: {other:?}"),
    };
    assert_eq!(
        entities
            .iter()
            .map(|e| e.name.clone().unwrap_or_default())
            .collect::<Vec<_>>(),
        vec!["First", "Second", "Third"],
    );
}

/// Play flips the gate; Stop puts back the world as it stood when play
/// began, so a play session cannot corrupt the authored scene.
#[test]
fn play_snapshots_the_world_and_stop_restores_it() {
    use kooch_core::run_state::Playing;

    let mut resources = ecs();
    let hero = match call(
        &mut resources,
        Method::Spawn {
            name: Some("Hero".into()),
            scene: None,
            parent: None,
        },
    ) {
        ResponseData::Spawned { entity } => entity,
        other => panic!("spawn: {other:?}"),
    };
    let transform_ty = std::any::type_name::<Transform>().to_owned();
    call(
        &mut resources,
        Method::AddComponent {
            entity: hero,
            component: transform_ty.clone(),
        },
    );

    assert!(!Playing::is_playing(&resources), "starts paused");
    call(&mut resources, Method::SetPlaying { playing: true });
    assert!(Playing::is_playing(&resources));

    // Stand in for what a gameplay system would do to the world.
    call(
        &mut resources,
        Method::SetField {
            entity: hero,
            component: transform_ty.clone(),
            field: "position".into(),
            value: ReflectValue::Vec3(glam::Vec3::splat(42.0)),
        },
    );

    call(&mut resources, Method::SetPlaying { playing: false });
    assert!(!Playing::is_playing(&resources));

    let entities = match call(&mut resources, Method::ListEntities { since: None }) {
        ResponseData::Entities { entities, .. } => entities,
        other => panic!("list: {other:?}"),
    };
    // The handle survives the round-trip: a client that mirrored this
    // world before play can still address the same entity after stop.
    assert_eq!(
        entities.iter().map(|e| e.id).collect::<Vec<_>>(),
        vec![hero],
        "entity identity churned across a play session"
    );
    let position = entities
        .iter()
        .find(|e| e.name.as_deref() == Some("Hero"))
        .expect("hero survived the restore")
        .components
        .iter()
        .find(|c| c.type_name == transform_ty)
        .and_then(|c| c.fields.iter().find(|(n, _)| n == "position"))
        .map(|(_, v)| v.clone());
    assert_eq!(
        position,
        Some(ReflectValue::Vec3(glam::Vec3::ZERO)),
        "play mutation leaked into the authored scene"
    );
}

/// A second Play must not overwrite the snapshot taken by the first,
/// or Stop would restore a world that already ran.
#[test]
fn repeated_play_keeps_the_original_snapshot() {
    use kooch_core::run_state::Playing;

    let mut resources = ecs();
    call(&mut resources, Method::SetPlaying { playing: true });
    call(
        &mut resources,
        Method::Spawn {
            name: Some("Spawned during play".into()),
            scene: None,
            parent: None,
        },
    );
    call(&mut resources, Method::SetPlaying { playing: true });
    call(&mut resources, Method::SetPlaying { playing: false });

    assert!(!Playing::is_playing(&resources));
    let entities = match call(&mut resources, Method::ListEntities { since: None }) {
        ResponseData::Entities { entities, .. } => entities,
        other => panic!("list: {other:?}"),
    };
    assert!(entities.is_empty(), "runtime spawn survived Stop");
}

/// The security property, asserted rather than assumed.
///
/// A running server must not be reachable over TCP. The old transport
/// bound 15703 on loopback, which any process — and any web page, via
/// `fetch` with a `text/plain` body and no CORS preflight — could reach
/// and drive. `SaveScene` writes to any path, so that was code execution
/// from visiting a page (#647).
///
/// This does not prove the whole class is closed; it proves the port that
/// was open is not.
#[test]
fn the_server_is_not_reachable_over_tcp() {
    use std::net::TcpStream;
    use std::time::Duration;

    let _server = RemoteServer::start(&test_socket_name()).expect("bind a socket");

    // The port the protocol used to live on.
    let addr = "127.0.0.1:15703".parse().unwrap();
    let refused = TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_err();
    assert!(
        refused,
        "something is listening on the old TCP port; the protocol must not be TCP-reachable"
    );
}

/// Where does the line framing stop working?
///
/// The reported failure — a snapshot that decodes as
/// `missing field \`id\`` — appeared once entities carried enough fields
/// to make the reply large. This grows the reply until it breaks, so the
/// limit is a number rather than a guess.
#[test]
fn a_large_snapshot_survives_the_framing() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let server = RemoteServer::start(&test_socket_name()).expect("bind");
    let socket = server.name().to_owned();

    let done = Arc::new(AtomicBool::new(false));
    let loop_done = Arc::clone(&done);
    let main_loop = std::thread::spawn(move || {
        let mut resources = ecs();
        while !loop_done.load(Ordering::Relaxed) {
            for item in server.take_pending() {
                let response = handle(&item.request, &mut resources);
                let _ = item.reply.send(response);
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    });

    let client = RemoteClient::new(&socket);

    // Each entity carries a Transform, so the reply grows steadily.
    for n in 0..400 {
        let e = client
            .spawn(Some(&format!("Entity{n}")), None, None)
            .expect("spawn");
        client
            .add_component(e, std::any::type_name::<Transform>())
            .expect("add");

        match client.list_entities() {
            Ok(entities) => assert_eq!(entities.len(), n + 1),
            Err(e) => panic!("the snapshot stopped decoding at {} entities: {e}", n + 1),
        }
    }

    done.store(true, Ordering::Relaxed);
    main_loop.join().unwrap();
}

/// Every field value has to survive JSON as **one line**: the protocol
/// frames messages by newline, so a value carrying a raw `\n` would cut
/// the message in half and the second half would decode as a stray
/// object — which is exactly the `missing field \`id\`` that was seen.
#[test]
fn no_field_value_serialises_with_a_raw_newline() {
    let values = [
        ReflectValue::F32(1.5),
        ReflectValue::String("plain".into()),
        ReflectValue::String("with\nnewline".into()),
        ReflectValue::Bool(true),
        ReflectValue::EntityRef(None),
    ];
    for v in &values {
        let json = serde_json::to_string(v).expect("serialises");
        assert!(
            !json.contains('\n'),
            "{v:?} serialised with a raw newline: {json}"
        );
    }
}

/// A non-finite float survives the wire.
///
/// JSON has no spelling for infinity or NaN, and `serde_json` does not
/// refuse — it writes `null`, which then fails to read back as a float.
/// `ReflectValue` writes the three that have no number as text instead;
/// infinity is how a joint spells "this motor has no ceiling", so it is
/// not a corner case.
#[test]
fn a_non_finite_float_survives_the_wire() {
    for value in [f32::INFINITY, f32::NEG_INFINITY, f32::NAN] {
        let json = serde_json::to_string(&ReflectValue::F32(value))
            .expect("serde_json writes null rather than erroring");
        let back: Result<ReflectValue, _> = serde_json::from_str(&json);
        assert!(
            back.is_ok(),
            "{value} serialised to {json} and could not be read back: {back:?}"
        );
    }
}

/// A queued request wakes a main loop that is allowed to sleep.
///
/// The listener parks on a reply only the main thread can produce. Once
/// that thread stops spinning between frames (#656), a request arriving
/// mid-sleep has to be what wakes it — otherwise the editor asking a
/// perfectly healthy project a question hangs until something unrelated
/// happens to produce a frame.
#[test]
fn a_queued_request_wakes_a_sleeping_main_loop() {
    use std::time::Duration;

    use kooch_core::frame_pacing::FrameWaker;

    let waker = FrameWaker::default();
    let (wake_tx, wake_rx) = std::sync::mpsc::channel::<()>();
    waker.set_notify(move || {
        let _ = wake_tx.send(());
    });

    let server = RemoteServer::start_waking(&test_socket_name(), waker.clone()).expect("bind");
    let socket = server.name().to_owned();

    let client = std::thread::spawn(move || {
        let name = socket
            .as_str()
            .to_ns_name::<GenericNamespaced>()
            .expect("valid name");
        let stream = Stream::connect(name).expect("connect");
        let mut conn = BufReader::new(stream);
        conn.get_mut()
            .write_all(b"{\"id\":7,\"method\":\"ping\"}\n")
            .unwrap();
        let mut response = String::new();
        conn.read_line(&mut response).unwrap();
        response
    });

    // Nothing has drained the queue yet — this is the sleeping loop, and
    // the only thing that can end the wait is the listener.
    wake_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("a queued request must wake the loop");
    assert!(
        waker.take_pending(),
        "the wake is recorded as well as signalled, so a loop that was \
         mid-frame when it landed does not sleep through it",
    );

    let mut resources = ecs();
    let mut answered = false;
    for _ in 0..2000 {
        for item in server.take_pending() {
            let response = handle(&item.request, &mut resources);
            item.reply.send(response).unwrap();
            answered = true;
        }
        if answered {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(answered, "the woken frame found nothing to answer");

    let response = client.join().unwrap();
    assert!(response.contains("\"kind\":\"pong\""), "body: {response}");
}

/// The project's open scenes travel with the snapshot the editor
/// already pulls every frame.
///
/// 🔴 This is the field the World panel draws its roots from. The
/// editor cannot answer it locally: its own `SceneManager` seeds an
/// unsaved scene with a random id, so without this the panel listed a
/// scene nothing belongs to and filed every mirrored entity under
/// "Unsaved" — the scene each one named was in nobody's list.
#[test]
fn the_open_scene_set_is_listed() {
    let mut resources = ecs();
    let mut manager = kooch_ecs::SceneManager::new();
    manager.set_current(std::path::PathBuf::from("assets/scenes/many_lights.scene"));
    let id = manager
        .active_id()
        .expect("new manager has an active scene");
    resources.insert(manager);

    let scenes = match call(&mut resources, Method::ListEntities { since: None }) {
        ResponseData::Entities { scenes, .. } => scenes.expect("the host has a SceneManager"),
        other => panic!("list: {other:?}"),
    };

    assert_eq!(scenes.len(), 1);
    assert_eq!(scenes[0].id, id, "the project's id, not one minted here");
    assert_eq!(
        scenes[0].path.as_deref(),
        Some("assets/scenes/many_lights.scene"),
    );
    assert!(scenes[0].active);
}

/// A host with no `SceneManager` says nothing, rather than saying no
/// scenes are open.
///
/// The editor replaces its list from this field, so the two have to be
/// distinguishable — answering with an empty list would blank the
/// World panel every frame.
#[test]
fn a_host_without_scenes_says_nothing() {
    let mut resources = ecs();
    match call(&mut resources, Method::ListEntities { since: None }) {
        ResponseData::Entities { scenes, .. } => assert_eq!(scenes, None),
        other => panic!("list: {other:?}"),
    }
}

/// Loading a *second* scene teaches the project's `SceneManager`.
///
/// 🔴 The handler used to go straight to `sync_scene_to_ecs`: the
/// entities arrived and the manager was told nothing, so it went on
/// describing the scene before this one.
///
/// The boot scene hides that. `SceneBootstrapPlugin` loads through the
/// manager, so a host that opens its startup scene and is never asked
/// for another looks perfectly correct — record and world agree because
/// neither has moved. Which is why this test loads twice: one load
/// passes with the bug in place.
#[test]
fn loading_a_second_scene_teaches_the_manager() {
    let mut resources = ecs();
    resources.insert(kooch_ecs::SceneManager::new());

    let dir = std::env::temp_dir().join("kooch_remote_load_scene");
    std::fs::create_dir_all(&dir).expect("temp dir");

    let mut write_scene = |name: &str, id: &str| {
        let path = dir.join(name);
        std::fs::write(
            &path,
            format!(r#"(id: "{id}", name: "{name}", version: "0.1.0", entities: [])"#),
        )
        .expect("write scene");
        path
    };
    let first = write_scene("station.scene", "ae0b881d-c3e2-49e1-ae19-cf8c3db5288e");
    let second = write_scene("hangar.scene", "019023f7-29d5-433e-98c8-e79461209106");

    let mut load = |resources: &mut Resources, path: &std::path::Path| {
        call(
            resources,
            Method::LoadScene {
                path: path.to_string_lossy().into_owned(),
            },
        );
    };
    load(&mut resources, &first);
    load(&mut resources, &second);

    let scenes = match call(&mut resources, Method::ListEntities { since: None }) {
        ResponseData::Entities { scenes, .. } => scenes.expect("the host has a SceneManager"),
        other => panic!("list: {other:?}"),
    };
    assert_eq!(scenes.len(), 1, "the second load replaced the first");
    assert_eq!(
        scenes[0].id,
        "019023f7-29d5-433e-98c8-e79461209106"
            .parse::<kooch_core::Guid>()
            .expect("a well-formed id"),
        "the project still named the scene it had left",
    );
    assert_eq!(
        scenes[0].path.as_deref(),
        Some(second.to_string_lossy().as_ref()),
    );
    assert!(scenes[0].active);

    let _ = std::fs::remove_file(&first);
    let _ = std::fs::remove_file(&second);
}

/// Saving over the wire writes one scene, and does not re-mint its id.
///
/// 🔴 `SaveScene` used to call `SceneDocument::from_ecs`:
/// `Capture::Everything` plus a fresh `Guid` for the document. Two
/// consequences, both silent. With more than one scene open it wrote
/// them all into the one file, so the next load spawned every entity
/// twice. And the id changed on every save, so anything that referred to
/// the scene by identity pointed at a file that no longer claimed it.
///
/// The engine has always had `from_ecs_scene`. The local editor path used
/// it, this one did not, and **Open Project always opens remote** — so
/// the wrong one was the one that ran.
#[test]
fn saving_writes_one_scene_and_keeps_its_id() {
    let mut resources = ecs();
    resources.insert(kooch_ecs::SceneManager::new());

    let dir = std::env::temp_dir().join("kooch_remote_save_scene");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let source = dir.join("station.scene");
    let id = "ae0b881d-c3e2-49e1-ae19-cf8c3db5288e";
    std::fs::write(
        &source,
        format!(r#"(id: "{id}", name: "Station", version: "0.1.0", entities: [])"#),
    )
    .expect("write scene");

    call(
        &mut resources,
        Method::LoadScene {
            path: source.to_string_lossy().into_owned(),
        },
    );

    let out = dir.join("written.scene");
    call(
        &mut resources,
        Method::SaveScene {
            path: out.to_string_lossy().into_owned(),
            scene: None,
        },
    );

    let written = kooch_ecs::scene::SceneDocument::load(&out).expect("reads back");
    assert_eq!(
        written.id,
        id.parse::<kooch_core::Guid>().expect("a well-formed id"),
        "the save minted a new identity for a scene that already had one",
    );

    let _ = std::fs::remove_file(&source);
    let _ = std::fs::remove_file(&out);
}

/// A host with no `SceneManager` refuses to save rather than writing the
/// whole world into the file.
#[test]
fn saving_without_a_manager_is_refused() {
    let mut resources = ecs();
    let out = std::env::temp_dir().join("kooch_remote_no_manager.scene");
    // Cleared first: the assertion below is "nothing was written", and a
    // leftover from an earlier run would fail a correct implementation.
    let _ = std::fs::remove_file(&out);
    let response = handle(
        &Request {
            id: 1,
            method: Method::SaveScene {
                path: out.to_string_lossy().into_owned(),
                scene: None,
            },
        },
        &mut resources,
    );
    assert!(
        matches!(response.payload, ResponsePayload::Error(_)),
        "wrote something without knowing which scene it was",
    );
    assert!(!out.exists(), "a refused save left a file behind");
}

/// An edit marks the scene it changed, and a save clears it.
///
/// 🔴 Nothing in the engine marked a scene dirty before this.
/// `SceneManager::mark_dirty` was called by its own tests and by nothing
/// else, so `dirty` was permanently `false`: the World panel's asterisk
/// could never appear and `any_dirty()` always answered "nothing to
/// lose". Nobody had seen the asterisk, so nobody noticed it was inert.
#[test]
fn an_edit_marks_the_scene_dirty() {
    let mut resources = ecs();
    resources.insert(kooch_ecs::SceneManager::new());

    let dir = std::env::temp_dir().join("kooch_remote_dirty");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("station.scene");
    std::fs::write(
        &path,
        r#"(id: "ae0b881d-c3e2-49e1-ae19-cf8c3db5288e", name: "Station", version: "0.1.0", entities: [])"#,
    )
    .expect("write scene");

    let dirty = |resources: &mut Resources| -> bool {
        match call(resources, Method::ListEntities { since: None }) {
            ResponseData::Entities { scenes, .. } => scenes.expect("open set")[0].dirty,
            other => panic!("list: {other:?}"),
        }
    };

    call(
        &mut resources,
        Method::LoadScene {
            path: path.to_string_lossy().into_owned(),
        },
    );
    assert!(!dirty(&mut resources), "a freshly loaded scene is clean");

    call(
        &mut resources,
        Method::Spawn {
            name: None,
            scene: None,
            parent: None,
        },
    );
    assert!(
        dirty(&mut resources),
        "spawning left the scene reading clean"
    );

    let out = dir.join("written.scene");
    call(
        &mut resources,
        Method::SaveScene {
            path: out.to_string_lossy().into_owned(),
            scene: None,
        },
    );
    assert!(!dirty(&mut resources), "the save did not clear the flag");

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&out);
}

/// The scene that changed is marked, not the one that happens to be
/// active.
///
/// With two open those are different, and marking the active one puts
/// the asterisk on the file nobody touched while leaving it off the one
/// they did.
#[test]
fn the_edited_scene_is_the_one_marked() {
    use kooch_ecs::SceneManager;

    let mut resources = ecs();
    let mut manager = SceneManager::new();
    let active = manager.active_id().expect("a scene");
    // A second scene, open but not active. Registered by hand: opening
    // one additively is not a remote method, and what is under test is
    // which of the two an edit marks.
    let elsewhere = kooch_core::Guid::new_v4();
    assert!(
        !manager.mark_scene_dirty(elsewhere),
        "a scene that is not open cannot be marked",
    );
    resources.insert(manager);

    // An entity belonging to no scene falls back to the active one.
    let entity = match call(
        &mut resources,
        Method::Spawn {
            name: None,
            scene: None,
            parent: None,
        },
    ) {
        ResponseData::Spawned { entity } => entity,
        other => panic!("spawn: {other:?}"),
    };
    let manager = resources.get::<SceneManager>().expect("manager");
    assert!(
        manager.scene(active).expect("open").dirty,
        "an unowned entity marks the scene that will adopt it",
    );
    let _ = entity;
}

/// A spawn lands where it was asked for, not in the active scene.
///
/// 🔴 Every spawn used to arrive in the active scene at the root — right
/// for a toolbar button, wrong for a menu opened on a scene or an entity
/// that is not the active one. The entity appears somewhere other than
/// where it was asked for, and the only sign is a row in the wrong group.
#[test]
fn a_spawn_lands_in_the_scene_it_names() {
    use kooch_ecs::SceneManager;

    let mut resources = ecs();
    let mut manager = SceneManager::new();
    let active = manager.active_id().expect("a scene");
    let elsewhere = manager.new_scene();
    assert!(manager.set_active(active), "put the active one back");
    resources.insert(manager);

    let spawned = |resources: &mut Resources, scene, parent| match call(
        resources,
        Method::Spawn {
            name: None,
            scene,
            parent,
        },
    ) {
        ResponseData::Spawned { entity } => entity,
        other => panic!("spawn: {other:?}"),
    };
    let home = |resources: &Resources, entity: kooch_remote::protocol::EntityId| {
        resources
            .get::<ComponentRegistry>()
            .and_then(|r| r.get_cpu::<kooch_ecs::SceneMember>())
            .and_then(|s| s.get(kooch_ecs::entity::Entity::from(entity)))
            .map(|m| m.scene)
    };

    let plain = spawned(&mut resources, None, None);
    assert_eq!(home(&resources, plain), Some(active), "unnamed went astray");

    let named = spawned(&mut resources, Some(elsewhere), None);
    assert_eq!(
        home(&resources, named),
        Some(elsewhere),
        "the scene it named was ignored for the active one",
    );

    // A parent already names the scene, so the child follows it even
    // though the request says nothing about scenes.
    let child = spawned(&mut resources, None, Some(named));
    assert_eq!(
        home(&resources, child),
        Some(elsewhere),
        "a child was authored into a scene its parent is not in",
    );

    // And a parent wins over a scene that disagrees: an entity's scene
    // IS its parent's, so honouring both would write the child to a file
    // its parent is not in.
    let contested = spawned(&mut resources, Some(active), Some(named));
    assert_eq!(
        home(&resources, contested),
        Some(elsewhere),
        "the scene field overrode the parent, splitting a tree across two files",
    );
}

/// A new scene opens beside the others and takes the spawn that asked
/// for it.
///
/// What right-clicking the World panel's empty space means: not "put
/// this somewhere" — there is no row under the pointer to name a
/// somewhere — but "start something new". An entity has to belong to a
/// scene, so opening one is what makes the gesture answerable.
#[test]
fn a_new_scene_opens_unsaved() {
    let mut resources = ecs();
    resources.insert(kooch_ecs::SceneManager::new());

    let opened = match call(&mut resources, Method::NewScene) {
        ResponseData::SceneOpened { scene } => scene,
        other => panic!("new_scene: {other:?}"),
    };

    let scenes = match call(&mut resources, Method::ListEntities { since: None }) {
        ResponseData::Entities { scenes, .. } => scenes.expect("open set"),
        other => panic!("list: {other:?}"),
    };
    assert_eq!(scenes.len(), 2, "it replaced the scene already open");
    let fresh = scenes.iter().find(|s| s.id == opened).expect("listed");
    assert_eq!(fresh.path, None, "an unsaved scene claimed a file");
    assert!(fresh.active, "new entities would not land in it");
    assert!(!fresh.dirty, "an empty scene has nothing to lose yet");

    let entity = match call(
        &mut resources,
        Method::Spawn {
            name: Some("First".into()),
            scene: Some(opened),
            parent: None,
        },
    ) {
        ResponseData::Spawned { entity } => entity,
        other => panic!("spawn: {other:?}"),
    };
    let home = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<kooch_ecs::SceneMember>())
        .and_then(|s| s.get(kooch_ecs::entity::Entity::from(entity)))
        .map(|m| m.scene);
    assert_eq!(home, Some(opened), "the entity did not join the new scene");
}

/// Reverting throws away one scene's edits and leaves the rest alone.
#[test]
fn a_revert_reads_the_file_back() {
    use kooch_ecs::SceneManager;

    let mut resources = ecs();
    resources.insert(SceneManager::new());

    let dir = std::env::temp_dir().join("kooch_remote_revert");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("station.scene");
    std::fs::write(
        &path,
        r#"(id: "ae0b881d-c3e2-49e1-ae19-cf8c3db5288e", name: "Station", version: "0.1.0", entities: [])"#,
    )
    .expect("write scene");

    call(
        &mut resources,
        Method::LoadScene {
            path: path.to_string_lossy().into_owned(),
        },
    );
    call(
        &mut resources,
        Method::Spawn {
            name: Some("Mistake".into()),
            scene: None,
            parent: None,
        },
    );

    let named = |resources: &mut Resources| -> Vec<String> {
        match call(resources, Method::ListEntities { since: None }) {
            ResponseData::Entities { entities, .. } => {
                entities.iter().filter_map(|e| e.name.clone()).collect()
            }
            other => panic!("list: {other:?}"),
        }
    };
    assert!(named(&mut resources).contains(&"Mistake".to_owned()));

    call(&mut resources, Method::RevertScene { scene: None });
    assert!(
        !named(&mut resources).contains(&"Mistake".to_owned()),
        "the edit survived a discard",
    );

    let scenes = match call(&mut resources, Method::ListEntities { since: None }) {
        ResponseData::Entities { scenes, .. } => scenes.expect("open set"),
        other => panic!("list: {other:?}"),
    };
    assert!(!scenes[0].dirty, "a reverted scene still read as edited");

    let _ = std::fs::remove_file(&path);
}

/// A scene that has never been saved refuses to revert.
///
/// 🔴 There is nothing to read back, and despawning its entities would
/// delete work rather than undo it — the one thing "discard changes"
/// must never be mistaken for.
#[test]
fn an_unsaved_scene_refuses_to_revert() {
    let mut resources = ecs();
    resources.insert(kooch_ecs::SceneManager::new());
    call(
        &mut resources,
        Method::Spawn {
            name: Some("Work".into()),
            scene: None,
            parent: None,
        },
    );

    let response = handle(
        &Request {
            id: 1,
            method: Method::RevertScene { scene: None },
        },
        &mut resources,
    );
    assert!(
        matches!(response.payload, ResponsePayload::Error(_)),
        "an unsaved scene reverted to nothing",
    );

    match call(&mut resources, Method::ListEntities { since: None }) {
        ResponseData::Entities { entities, .. } => assert_eq!(
            entities.len(),
            1,
            "the refusal despawned the work it could not restore",
        ),
        other => panic!("list: {other:?}"),
    }
}

/// Moving an entity between two rows makes it their sibling, and takes
/// it out of whatever parent it was in.
#[test]
fn a_move_reorders_and_unparents() {
    let mut resources = ecs();
    resources.insert(kooch_ecs::SceneManager::new());

    let spawn = |resources: &mut Resources, name: &str, parent| match call(
        resources,
        Method::Spawn {
            name: Some(name.to_owned()),
            scene: None,
            parent,
        },
    ) {
        ResponseData::Spawned { entity } => entity,
        other => panic!("spawn: {other:?}"),
    };
    let a = spawn(&mut resources, "A", None);
    let b = spawn(&mut resources, "B", None);
    let c = spawn(&mut resources, "C", None);
    // D starts inside A.
    let d = spawn(&mut resources, "D", Some(a));

    let order = |resources: &Resources, e: kooch_remote::protocol::EntityId| {
        resources
            .get::<ComponentRegistry>()
            .and_then(|r| r.get_cpu::<kooch_ecs::Order>())
            .and_then(|s| s.get(kooch_ecs::entity::Entity::from(e)))
            .map(|o| o.value)
    };
    let parent_of = |resources: &Resources, e: kooch_remote::protocol::EntityId| {
        resources
            .get::<ComponentRegistry>()
            .and_then(|r| r.get_cpu::<kooch_ecs::hierarchy::Parent>())
            .and_then(|s| s.get(kooch_ecs::entity::Entity::from(e)))
            .map(|p| p.entity)
    };
    assert_eq!(
        parent_of(&resources, d),
        Some(kooch_ecs::entity::Entity::from(a)),
        "D did not start inside A",
    );

    // Drop D in the gap between B and C: a root, between them.
    call(
        &mut resources,
        Method::MoveEntity {
            entity: d,
            parent: None,
            before: Some(c),
        },
    );

    assert_eq!(parent_of(&resources, d), None, "D stayed inside A");
    let (oa, ob, od, oc) = (
        order(&resources, a),
        order(&resources, b),
        order(&resources, d),
        order(&resources, c),
    );
    assert!(
        oa < ob && ob < od && od < oc,
        "expected A < B < D < C, got {oa:?} {ob:?} {od:?} {oc:?}",
    );
}

/// Moving an entity into its own subtree is refused, rather than
/// detaching that subtree from the world.
#[test]
fn a_move_into_itself_is_refused() {
    let mut resources = ecs();
    resources.insert(kooch_ecs::SceneManager::new());
    let spawn = |resources: &mut Resources, parent| match call(
        resources,
        Method::Spawn {
            name: None,
            scene: None,
            parent,
        },
    ) {
        ResponseData::Spawned { entity } => entity,
        other => panic!("spawn: {other:?}"),
    };
    let root = spawn(&mut resources, None);
    let child = spawn(&mut resources, Some(root));

    let response = handle(
        &Request {
            id: 1,
            method: Method::MoveEntity {
                entity: root,
                parent: Some(child),
                before: None,
            },
        },
        &mut resources,
    );
    assert!(matches!(response.payload, ResponsePayload::Error(_)));
}
