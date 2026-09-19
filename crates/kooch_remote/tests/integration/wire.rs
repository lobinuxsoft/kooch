//! Play and the wire: snapshots, the socket, framing, odd values, waking the main loop.

use super::*;

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

/// The security property: a running server is not reachable over TCP — the old loopback port let a
/// web page drive `SaveScene` (#647).
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

/// Where the line framing stops working: grows the reply until it breaks, so the limit is a number.
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

/// Every field value survives JSON as one line, since a raw newline would split the message into a
/// stray object.
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

/// A non-finite float survives the wire: `ReflectValue` writes infinity and NaN as text, because
/// `serde_json` writes `null`.
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

/// A queued request wakes a main loop allowed to sleep (#656), or a healthy project hangs the
/// editor until an unrelated frame.
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
