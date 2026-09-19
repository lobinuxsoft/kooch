//! Spawning, reparenting and duplicating on the project's side.

use super::*;

/// The bug: `Spawn ▸ 3D Object` was dropped on the floor in remote mode, because `classify` had no
/// arm for it and nothing else claimed it. Silently — no entity, no error.
#[test]
fn spawn_mesh_builds_the_entity_on_the_project() {
    use kooch_ecs::mesh_renderer::MeshRenderer;

    let server = RemoteServer::start(&test_socket_name()).expect("bind");
    let socket = server.name().to_owned();
    let done = Arc::new(AtomicBool::new(false));
    let loop_done = Arc::clone(&done);

    // The project: a world that can hold a MeshRenderer.
    let main_loop = std::thread::spawn(move || {
        let mut res = ecs();
        {
            let reg = res.get_mut::<ComponentRegistry>().unwrap();
            reg.register_cpu_reflected::<MeshRenderer>();
        }
        while !loop_done.load(Ordering::Relaxed) {
            for item in server.take_pending() {
                let resp = handle(&item.request, &mut res);
                let _ = item.reply.send(resp);
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    });

    let mut editor = editor_with_assets();
    let mut state = RemoteState::new();
    state.session = Some(RemoteSession::attach(&socket));
    for _ in 0..200 {
        if state.session.as_mut().unwrap().poll_ready() == ConnectionState::Connected {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(state.is_connected(), "did not connect");
    editor.insert(state);

    let action = EditorAction::SpawnMesh {
        path: std::path::PathBuf::from("meshes/suzanne.glb"),
        name: "Suzanne".to_owned(),
    };
    assert!(
        dispatch(&mut editor, &action),
        "the remote sink still does not own SpawnMesh"
    );

    // Ask the project what it has.
    let client = RemoteClient::new(&socket);
    let entities = client.list_entities().unwrap();
    let spawned = entities
        .iter()
        .find(|e| {
            e.components
                .iter()
                .any(|c| c.type_name.ends_with("MeshRenderer"))
        })
        .expect("the project has no entity with a MeshRenderer");

    for expected in ["Name", "Transform", "MeshRenderer"] {
        assert!(
            spawned
                .components
                .iter()
                .any(|c| c.type_name.ends_with(expected)),
            "the spawned entity has no {expected}: {:?}",
            spawned
                .components
                .iter()
                .map(|c| &c.type_name)
                .collect::<Vec<_>>()
        );
    }

    let mesh = spawned
        .components
        .iter()
        .find(|c| c.type_name.ends_with("MeshRenderer"))
        .and_then(|c| c.fields.iter().find(|(n, _)| n == "mesh"))
        .map(|(_, v)| v.clone())
        .expect("MeshRenderer has no mesh field");
    assert!(
        matches!(mesh, ReflectValue::AssetRef { guid: Some(_), .. }),
        "the mesh reference did not reach the project: {mesh:?}"
    );

    done.store(true, Ordering::Relaxed);
    main_loop.join().unwrap();
}

/// An unresolvable path is claimed and logged, not passed through to the local path where it would
/// spawn into the mirror — which the next refresh would wipe, looking like a flicker.
#[test]
fn an_unresolvable_mesh_is_still_owned_by_the_remote_sink() {
    let mut editor = editor_with_assets();
    editor.insert(RemoteState::new());

    let action = EditorAction::SpawnMesh {
        path: std::path::PathBuf::from("meshes/does_not_exist.glb"),
        name: "Ghost".to_owned(),
    };
    assert!(dispatch(&mut editor, &action));
}

/// What was reported: a light spawned remotely arrived with a `Name` and nothing else.
#[test]
fn spawn_carries_its_extra_components_over_the_wire() {
    use kooch_ecs::directional_light::DirectionalLight;

    let server = RemoteServer::start(&test_socket_name()).expect("bind");
    let socket = server.name().to_owned();
    let done = Arc::new(AtomicBool::new(false));
    let loop_done = Arc::clone(&done);

    let main_loop = std::thread::spawn(move || {
        let mut res = ecs();
        {
            let reg = res.get_mut::<ComponentRegistry>().unwrap();
            reg.register_cpu_reflected::<DirectionalLight>();
        }
        while !loop_done.load(Ordering::Relaxed) {
            for item in server.take_pending() {
                let resp = handle(&item.request, &mut res);
                let _ = item.reply.send(resp);
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    });

    let mut editor = ecs();
    {
        let reg = editor.get_mut::<ComponentRegistry>().unwrap();
        reg.register_cpu_reflected::<DirectionalLight>();
    }
    let mut state = RemoteState::new();
    state.session = Some(RemoteSession::attach(&socket));
    for _ in 0..200 {
        if state.session.as_mut().unwrap().poll_ready() == ConnectionState::Connected {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(state.is_connected(), "did not connect");
    editor.insert(state);

    let action = EditorAction::Spawn {
        extra: vec![std::any::TypeId::of::<DirectionalLight>()],
        name: Some("Directional Light".to_owned()),
        into: crate::actions::SpawnTarget::Active,
    };
    assert!(dispatch(&mut editor, &action));

    let client = RemoteClient::new(&socket);
    let entities = client.list_entities().unwrap();
    let light = entities
        .iter()
        .find(|e| {
            e.components
                .iter()
                .any(|c| c.type_name.ends_with("DirectionalLight"))
        })
        .unwrap_or_else(|| {
            panic!(
                "the light component never reached the project; got {:?}",
                entities
                    .iter()
                    .map(|e| e
                        .components
                        .iter()
                        .map(|c| &c.type_name)
                        .collect::<Vec<_>>())
                    .collect::<Vec<_>>()
            )
        });

    // A light with no Transform has no position and no direction, so this is not a nice-to-have —
    // it is the difference between a light and an inert entity.
    for expected in ["Name", "Transform", "DirectionalLight"] {
        assert!(
            light
                .components
                .iter()
                .any(|c| c.type_name.ends_with(expected)),
            "the spawned light has no {expected}: {:?}",
            light
                .components
                .iter()
                .map(|c| &c.type_name)
                .collect::<Vec<_>>()
        );
    }

    done.store(true, Ordering::Relaxed);
    main_loop.join().unwrap();
}

/// #595, hole one: `Reparent` had no `classify` arm, fell through to the
/// local path, mutated the *mirror*, and silently reverted half a second
/// later when the refresh rebuilt parent links from the project.
#[test]
fn reparent_reaches_the_project() {
    use kooch_ecs::hierarchy::Parent;

    let server = RemoteServer::start(&test_socket_name()).expect("bind");
    let socket = server.name().to_owned();
    let done = Arc::new(AtomicBool::new(false));
    let loop_done = Arc::clone(&done);

    let main_loop = std::thread::spawn(move || {
        let mut res = ecs();
        {
            let reg = res.get_mut::<ComponentRegistry>().unwrap();
            reg.register_cpu_reflected::<Parent>();
            reg.register_cpu_reflected::<kooch_ecs::hierarchy::Children>();
        }
        // Two root entities for the editor to relate.
        for name in ["Parent", "Child"] {
            handle(
                &Request {
                    id: 1,
                    notify: false,
                    method: Method::Spawn {
                        name: Some(name.to_owned()),
                        scene: None,
                        parent: None,
                    },
                },
                &mut res,
            );
        }
        while !loop_done.load(Ordering::Relaxed) {
            for item in server.take_pending() {
                let resp = handle(&item.request, &mut res);
                let _ = item.reply.send(resp);
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    });

    let mut editor = ecs();
    {
        let reg = editor.get_mut::<ComponentRegistry>().unwrap();
        reg.register_cpu_reflected::<Parent>();
        reg.register_cpu_reflected::<kooch_ecs::hierarchy::Children>();
    }
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
    state.mirror.apply(&snapshot, &mut editor);
    assert!(snapshot.len() >= 2, "the project has no pair to relate");
    let parent = state.mirror.local_of(snapshot[0].id).expect("mirrored");
    let child = state.mirror.local_of(snapshot[1].id).expect("mirrored");
    editor.insert(state);

    assert!(dispatch(
        &mut editor,
        &EditorAction::Reparent {
            entity: child,
            new_parent: Some(parent),
        }
    ));

    // The project itself has to report the relationship — not the mirror,
    // which is exactly what used to be mutated instead.
    let client = RemoteClient::new(&socket);
    let entities = client.list_entities().unwrap();
    let child_remote = entities
        .iter()
        .find(|e| e.id == snapshot[1].id)
        .expect("child gone");
    assert_eq!(
        child_remote.parent,
        Some(snapshot[0].id),
        "the reparent never reached the project"
    );

    // And unparenting travels the same way.
    assert!(dispatch(
        &mut editor,
        &EditorAction::Reparent {
            entity: child,
            new_parent: None,
        }
    ));
    let entities = client.list_entities().unwrap();
    let child_remote = entities.iter().find(|e| e.id == snapshot[1].id).unwrap();
    assert_eq!(child_remote.parent, None, "unparenting did not travel");

    done.store(true, Ordering::Relaxed);
    main_loop.join().unwrap();
}

/// #595, hole two: `Duplicate` was claimed by nothing at all — not
/// `classify`, not `dispatch`, not `apply_non_ecs_action` — so in remote
/// mode it was a silent no-op.
#[test]
fn duplicate_creates_a_copy_on_the_project() {
    let server = RemoteServer::start(&test_socket_name()).expect("bind");
    let socket = server.name().to_owned();
    let done = Arc::new(AtomicBool::new(false));
    let loop_done = Arc::clone(&done);

    let main_loop = std::thread::spawn(move || {
        let mut res = ecs();
        let entity = {
            let mut commands = res.remove::<Commands>().unwrap();
            let e = commands.spawn(&mut res).id();
            commands.apply(&mut res);
            res.insert(commands);
            e
        };
        if let Some(reg) = res.get_mut::<ComponentRegistry>() {
            reg.insert_default_reflected(&TypeId::of::<Transform>(), entity);
        }
        let empty = res
            .get_mut::<kooch_ecs::archetype_registry::ArchetypeRegistry>()
            .unwrap()
            .get_or_create(Default::default());
        let archetypes = res
            .get_mut::<kooch_ecs::archetype_registry::ArchetypeRegistry>()
            .unwrap();
        archetypes.register_entity(entity, empty);
        let next = archetypes.archetype_after_add::<Transform>(empty);
        archetypes.register_entity(entity, next);

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
    state.mirror.apply(&snapshot, &mut editor);
    let source = state.mirror.local_of(snapshot[0].id).expect("mirrored");

    // Give the source a distinctive value so the copy can be told apart
    // from an empty entity that merely exists.
    if let Some(reg) = editor.get_mut::<ComponentRegistry>() {
        let _ = reg.reflect_set_field(
            &TypeId::of::<Transform>(),
            source,
            "position",
            ReflectValue::Vec3(glam::Vec3::new(3.0, 4.0, 5.0)),
        );
    }
    editor.insert(state);

    let before = RemoteClient::new(&socket).list_entities().unwrap().len();
    assert!(dispatch(&mut editor, &EditorAction::Duplicate(source)));

    let entities = RemoteClient::new(&socket).list_entities().unwrap();
    assert_eq!(
        entities.len(),
        before + 1,
        "no copy was created on the project"
    );
    let copy = entities
        .iter()
        .find(|e| e.id != snapshot[0].id)
        .expect("copy not found");
    let position = copy
        .components
        .iter()
        .find(|c| c.type_name.ends_with("Transform"))
        .and_then(|c| c.fields.iter().find(|(n, _)| n == "position"))
        .map(|(_, v)| v.clone());
    assert_eq!(
        position,
        Some(ReflectValue::Vec3(glam::Vec3::new(3.0, 4.0, 5.0))),
        "the copy did not carry the source's field values"
    );

    done.store(true, Ordering::Relaxed);
    main_loop.join().unwrap();
}
