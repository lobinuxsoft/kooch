//! Requests end to end: spawning, fields, typed errors, the bridge, listing order.

use super::*;

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

/// A spawned entity carries `Name` and `Transform` with or without a name, as the local spawn does
/// — or the Inspector cannot rename it.
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
            notify: false,
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

/// The wire format, exercised raw without `RemoteClient`, so client and server cannot drift into
/// agreeing on something unspecified.
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

    // #645 — every call records transport and decode cost, so the editor can tell which half is the
    // bill.
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
