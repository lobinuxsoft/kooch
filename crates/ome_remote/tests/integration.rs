//! End-to-end tests: `handle` against a live ECS, and a full HTTP
//! round-trip through the listener thread and main-loop bridge.

use std::io::{BufRead, BufReader, Write};

use interprocess::local_socket::traits::Stream as _;
use interprocess::local_socket::{GenericNamespaced, Stream, ToNsName};

use ome_core::resource::Resources;
use ome_ecs::allocator::EntityAllocator;
use ome_ecs::archetype_registry::ArchetypeRegistry;
use ome_ecs::commands::Commands;
use ome_ecs::component::{ComponentNames, ComponentRegistry};
use ome_ecs::dynamic_components::DynamicComponents;
use ome_ecs::name::Name;
use ome_ecs::query::AccessTracker;
use ome_ecs::reflect::ReflectValue;
use ome_ecs::transform::Transform;

use ome_remote::client::RemoteClient;
use ome_remote::handlers::handle;
use ome_remote::protocol::{Method, Request, ResponseData, ResponsePayload};
use ome_remote::server::RemoteServer;

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
        },
    ) {
        ResponseData::Spawned { entity } => entity,
        other => panic!("expected Spawned, got {other:?}"),
    };

    // It shows up in the listing with its name.
    let listed = match call(&mut resources, Method::ListEntities) {
        ResponseData::Entities { entities } => entities,
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
    let listed = match call(&mut resources, Method::ListEntities) {
        ResponseData::Entities { entities } => entities,
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
    let entity = match call(&mut resources, Method::Spawn { name: None }) {
        ResponseData::Spawned { entity } => entity,
        other => panic!("{other:?}"),
    };

    let entities = match call(&mut resources, Method::ListEntities) {
        ResponseData::Entities { entities } => entities,
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
    let entity = match call(&mut resources, Method::Spawn { name: None }) {
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
        ResponsePayload::Error(ome_remote::protocol::RemoteError::UnknownComponent {
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
        "ome_test_{}_{}_{}.sock",
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

    let hero = client.spawn(Some("Hero")).expect("spawn");
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
            ome_remote::ClientError::Remote(
                ome_remote::protocol::RemoteError::UnknownComponent { .. }
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

    let entities = match call(&mut resources, Method::ListEntities) {
        ResponseData::Entities { entities } => entities,
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
    use ome_core::run_state::Playing;

    let mut resources = ecs();
    let hero = match call(
        &mut resources,
        Method::Spawn {
            name: Some("Hero".into()),
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

    let entities = match call(&mut resources, Method::ListEntities) {
        ResponseData::Entities { entities } => entities,
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
    use ome_core::run_state::Playing;

    let mut resources = ecs();
    call(&mut resources, Method::SetPlaying { playing: true });
    call(
        &mut resources,
        Method::Spawn {
            name: Some("Spawned during play".into()),
        },
    );
    call(&mut resources, Method::SetPlaying { playing: true });
    call(&mut resources, Method::SetPlaying { playing: false });

    assert!(!Playing::is_playing(&resources));
    let entities = match call(&mut resources, Method::ListEntities) {
        ResponseData::Entities { entities } => entities,
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
        let e = client.spawn(Some(&format!("Entity{n}"))).expect("spawn");
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

    use ome_core::frame_pacing::FrameWaker;

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
