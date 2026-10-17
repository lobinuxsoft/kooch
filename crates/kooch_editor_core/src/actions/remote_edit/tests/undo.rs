//! Undoing remote edits: fields, despawns, pastes.

use super::*;

/// The bug #811 was filed for: with a project open, Ctrl+Z was discarded on the first line of
/// `dispatch` and the field kept the value it had just been given.
#[test]
fn an_undone_field_goes_back() {
    let transform_ty = std::any::type_name::<Transform>();
    let server = RemoteServer::start(&test_socket_name()).expect("bind");
    let socket = server.name().to_owned();
    let done = Arc::new(AtomicBool::new(false));
    let loop_done = Arc::clone(&done);

    let main_loop = std::thread::spawn(move || {
        let mut res = ecs();
        let hero = match handle(
            &Request {
                id: 0,
                notify: false,
                method: Method::Spawn {
                    name: Some("Hero".into()),
                    scene: None,
                    parent: None,
                },
            },
            &mut res,
        )
        .payload
        {
            kooch_remote::protocol::ResponsePayload::Result(
                kooch_remote::protocol::ResponseData::Spawned { entity },
            ) => entity,
            _ => panic!("spawn"),
        };
        handle(
            &Request {
                id: 1,
                notify: false,
                method: Method::AddComponent {
                    entity: hero,
                    component: transform_ty.into(),
                },
            },
            &mut res,
        );
        while !loop_done.load(Ordering::Relaxed) {
            for item in server.take_pending() {
                let resp = handle(&item.request, &mut res);
                let _ = item.reply.send(resp);
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    });

    let mut editor = ecs();
    let mut state = RemoteState::new();
    state.session = Some(RemoteSession::attach(&socket));
    for _ in 0..200 {
        if state.session.as_mut().unwrap().poll_ready() == ConnectionState::Connected {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(state.is_connected(), "did not connect");
    let snapshot = state.session.as_ref().unwrap().snapshot().to_vec();
    state.mirror.apply(&snapshot, None, &mut editor);
    let local = state.mirror.local_of(snapshot[0].id).expect("mirrored");
    let comp = editor
        .get_mut::<ComponentNames>()
        .unwrap()
        .intern(transform_ty);
    editor.insert(state);

    let moved = glam::Vec3::new(9.0, 9.0, 9.0);
    assert!(dispatch(
        &mut editor,
        &EditorAction::SetField {
            entity: local,
            component: comp,
            field: "position".into(),
            value: ReflectValue::Vec3(moved),
        }
    ));
    assert_eq!(position(&socket), Some(ReflectValue::Vec3(moved)));

    assert!(dispatch(
        &mut editor,
        &EditorAction::Undo(crate::history::Document::World)
    ));
    assert_eq!(
        position(&socket),
        Some(ReflectValue::Vec3(glam::Vec3::ZERO)),
        "the undo did not reach the project",
    );

    // And back again — redo is the same machinery run the other way.
    assert!(dispatch(
        &mut editor,
        &EditorAction::Redo(crate::history::Document::World)
    ));
    assert_eq!(
        position(&socket),
        Some(ReflectValue::Vec3(moved)),
        "the redo did not reach the project",
    );

    done.store(true, Ordering::Relaxed);
    main_loop.join().unwrap();
}

/// Undoing a despawn is a creation: the entity comes back with its
/// components and values, under a new id the project hands out.
#[test]
fn an_undone_despawn_rebuilds_it() {
    let transform_ty = std::any::type_name::<Transform>();
    let server = RemoteServer::start(&test_socket_name()).expect("bind");
    let socket = server.name().to_owned();
    let done = Arc::new(AtomicBool::new(false));
    let loop_done = Arc::clone(&done);

    let main_loop = std::thread::spawn(move || {
        let mut res = ecs();
        let hero = match handle(
            &Request {
                id: 0,
                notify: false,
                method: Method::Spawn {
                    name: Some("Hero".into()),
                    scene: None,
                    parent: None,
                },
            },
            &mut res,
        )
        .payload
        {
            kooch_remote::protocol::ResponsePayload::Result(
                kooch_remote::protocol::ResponseData::Spawned { entity },
            ) => entity,
            _ => panic!("spawn"),
        };
        handle(
            &Request {
                id: 1,
                notify: false,
                method: Method::AddComponent {
                    entity: hero,
                    component: transform_ty.into(),
                },
            },
            &mut res,
        );
        handle(
            &Request {
                id: 2,
                notify: false,
                method: Method::SetField {
                    entity: hero,
                    component: transform_ty.into(),
                    field: "position".into(),
                    value: ReflectValue::Vec3(glam::Vec3::new(1.0, 2.0, 3.0)),
                },
            },
            &mut res,
        );
        while !loop_done.load(Ordering::Relaxed) {
            for item in server.take_pending() {
                let resp = handle(&item.request, &mut res);
                let _ = item.reply.send(resp);
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    });

    let mut editor = ecs();
    let mut state = RemoteState::new();
    state.session = Some(RemoteSession::attach(&socket));
    for _ in 0..200 {
        if state.session.as_mut().unwrap().poll_ready() == ConnectionState::Connected {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(state.is_connected(), "did not connect");
    let snapshot = state.session.as_ref().unwrap().snapshot().to_vec();
    state.mirror.apply(&snapshot, None, &mut editor);
    let local = state.mirror.local_of(snapshot[0].id).expect("mirrored");
    editor.insert(state);

    assert!(dispatch(&mut editor, &EditorAction::Despawn(local)));
    assert!(
        RemoteClient::new(&socket)
            .list_entities()
            .unwrap()
            .is_empty(),
        "the despawn did not reach the project",
    );

    assert!(dispatch(
        &mut editor,
        &EditorAction::Undo(crate::history::Document::World)
    ));
    let entities = RemoteClient::new(&socket).list_entities().unwrap();
    assert_eq!(entities.len(), 1, "the entity did not come back");
    assert_eq!(
        position(&socket),
        Some(ReflectValue::Vec3(glam::Vec3::new(1.0, 2.0, 3.0))),
        "it came back without its values",
    );

    done.store(true, Ordering::Relaxed);
    main_loop.join().unwrap();
}

/// The first entity's `Transform.position`, as the project holds it.
fn position(socket: &str) -> Option<ReflectValue> {
    RemoteClient::new(socket)
        .list_entities()
        .unwrap()
        .first()?
        .components
        .iter()
        .find(|c| c.type_name.ends_with("Transform"))
        .and_then(|c| c.fields.iter().find(|(n, _)| n == "position"))
        .map(|(_, v)| v.clone())
}

/// Ctrl+V builds the clipboard on the project's side, and Ctrl+Z takes
/// back exactly what it built — not the entity it was copied from.
#[test]
fn a_paste_is_built_and_undone() {
    let transform_ty = std::any::type_name::<Transform>();
    let server = RemoteServer::start(&test_socket_name()).expect("bind");
    let socket = server.name().to_owned();
    let done = Arc::new(AtomicBool::new(false));
    let loop_done = Arc::clone(&done);

    let main_loop = std::thread::spawn(move || {
        let mut res = ecs();
        let hero = match handle(
            &Request {
                id: 0,
                notify: false,
                method: Method::Spawn {
                    name: Some("Hero".into()),
                    scene: None,
                    parent: None,
                },
            },
            &mut res,
        )
        .payload
        {
            kooch_remote::protocol::ResponsePayload::Result(
                kooch_remote::protocol::ResponseData::Spawned { entity },
            ) => entity,
            _ => panic!("spawn"),
        };
        handle(
            &Request {
                id: 1,
                notify: false,
                method: Method::AddComponent {
                    entity: hero,
                    component: transform_ty.into(),
                },
            },
            &mut res,
        );
        while !loop_done.load(Ordering::Relaxed) {
            for item in server.take_pending() {
                let resp = handle(&item.request, &mut res);
                let _ = item.reply.send(resp);
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    });

    let mut editor = ecs();
    let mut state = RemoteState::new();
    state.session = Some(RemoteSession::attach(&socket));
    for _ in 0..200 {
        if state.session.as_mut().unwrap().poll_ready() == ConnectionState::Connected {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(state.is_connected(), "did not connect");
    let snapshot = state.session.as_ref().unwrap().snapshot().to_vec();
    state.mirror.apply(&snapshot, None, &mut editor);
    let source = state.mirror.local_of(snapshot[0].id).expect("mirrored");
    editor.insert(state);

    // What Ctrl+C leaves behind, without going through the panel that fills it.
    let mut clipboard = crate::clipboard::EntityClipboard::default();
    clipboard.set(vec![crate::actions::entity_state::capture_tree(
        &editor, source,
    )]);
    editor.insert(clipboard);

    assert!(dispatch(
        &mut editor,
        &EditorAction::PasteEntities {
            into: crate::actions::SpawnTarget::Active,
        }
    ));
    let entities = RemoteClient::new(&socket).list_entities().unwrap();
    assert_eq!(entities.len(), 2, "the paste did not reach the project");
    assert!(
        entities.iter().any(
            |e| e.components.iter().any(|c| c.type_name.ends_with("Name")
                && c.fields
                    .iter()
                    .any(|(_, v)| *v == ReflectValue::String("Hero Copy".to_owned())))
        ),
        "the pasted entity was not named after its source",
    );

    assert!(dispatch(
        &mut editor,
        &EditorAction::Undo(crate::history::Document::World)
    ));
    let after = RemoteClient::new(&socket).list_entities().unwrap();
    assert_eq!(after.len(), 1, "undoing the paste took the wrong entity");
    assert_eq!(after[0].id, snapshot[0].id, "it took the original");

    done.store(true, Ordering::Relaxed);
    main_loop.join().unwrap();
}

/// 🔴 The gizmo writes the new geometry onto the component and *then* emits `BlockEdit`, so a
/// before-state read from the world is the edit that was just made. Undo would write it back and
/// look like it did nothing.
#[test]
fn a_block_edit_keeps_its_before() {
    use kooch_ecs::reflect::Reflect;

    let mut editor = ecs();
    editor
        .get_mut::<ComponentRegistry>()
        .unwrap()
        .register_cpu_reflected::<kooch_blockmesh::Block>();

    let mut mirror = crate::remote_mirror::RemoteMirror::new();
    let snapshot = vec![kooch_remote::protocol::EntitySnapshot {
        id: kooch_remote::protocol::EntityId {
            index: 0,
            generation: 0,
        },
        name: Some("Block".into()),
        parent: None,
        scene: None,
        components: vec![kooch_remote::protocol::ComponentSnapshot {
            type_name: std::any::type_name::<kooch_blockmesh::Block>().into(),
            fields: Vec::new(),
        }],
    }];
    mirror.apply(&snapshot, None, &mut editor);
    let local = mirror.local_of(snapshot[0].id).expect("mirrored");

    let before = kooch_blockmesh::BlockMesh::cuboid(glam::Vec3::splat(0.5));
    let after = kooch_blockmesh::BlockMesh::cuboid(glam::Vec3::splat(2.0));
    // The world already holds `after`, which is the whole trap.
    crate::block_edit::set_shape(&mut editor, local, &after);

    let inverse = crate::actions::remote_undo::capture_before(
        &EditorAction::BlockEdit {
            entity: local,
            before: Box::new(before.clone()),
            after: Box::new(after.clone()),
        },
        &editor,
        &mirror,
    )
    .expect("a block edit is undoable");

    let crate::actions::remote_undo::Inverse::SetField {
        before: recorded, ..
    } = inverse
    else {
        panic!("a block edit undoes as a field edit");
    };
    assert_eq!(
        recorded,
        crate::actions::remote_edit::mesh_value(&before).unwrap(),
        "the undo would write back the edit it was meant to reverse",
    );
}
