//! End-to-end tests: `handle` against a live ECS, and a full HTTP
//! round-trip through the listener thread and main-loop bridge.

use std::io::{Read, Write};
use std::net::TcpStream;

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

#[test]
fn http_ping_round_trips_through_the_bridge() {
    // Bind an ephemeral-ish port; retry a few offsets if taken.
    let server = (0..8)
        .find_map(|i| RemoteServer::start(17700 + i).ok())
        .expect("bind a port");
    let port = server.port();

    // Client thread: POST a ping and capture the HTTP response body.
    let client = std::thread::spawn(move || {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let body = r#"{"id":42,"method":"ping"}"#;
        let request = format!(
            "POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(request.as_bytes()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
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
    assert!(response.contains("200 OK"), "status line: {response}");
    assert!(response.contains("\"kind\":\"pong\""), "body: {response}");
    assert!(response.contains("\"id\":42"), "id echoed: {response}");
}

#[test]
fn client_drives_server_end_to_end() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let server = (0..8)
        .find_map(|i| RemoteServer::start(17720 + i).ok())
        .expect("bind a port");
    let port = server.port();

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
    let client = RemoteClient::new(port);
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

    // A schema call reaches the registry.
    let schema = client.get_schema().expect("schema");
    assert!(schema.iter().any(|c| c.type_name.ends_with("Transform")));

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

    // The restore respawns entities, so look the value up by name.
    let entities = match call(&mut resources, Method::ListEntities) {
        ResponseData::Entities { entities } => entities,
        other => panic!("list: {other:?}"),
    };
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
