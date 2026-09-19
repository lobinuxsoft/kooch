//! Which edits reach the project's server, and which fall through to the local world.

use super::*;

/// A `SetField` issued in the editor lands on the project's server.
#[test]
fn set_field_routes_to_the_server() {
    let transform_ty = std::any::type_name::<Transform>();

    // Server side: a project with one Transform-bearing entity.
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

    // Editor side: connect, mirror, then issue an edit.
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
    let remote_id = snapshot[0].id;
    let local = state.mirror.local_of(remote_id).expect("mirrored");
    let comp = editor
        .get_mut::<ComponentNames>()
        .unwrap()
        .intern(transform_ty);
    editor.insert(state);
    let mut editor = editor;

    let action = EditorAction::SetField {
        entity: local,
        component: comp,
        field: "position".into(),
        value: ReflectValue::Vec3(glam::Vec3::new(9.0, 9.0, 9.0)),
    };
    assert!(
        dispatch(&mut editor, &action),
        "remote dispatch should own SetField"
    );

    // The server now holds the edited value.
    let client = RemoteClient::new(&socket);
    let entities = client.list_entities().unwrap();
    let pos = entities[0]
        .components
        .iter()
        .find(|c| c.type_name.ends_with("Transform"))
        .and_then(|c| c.fields.iter().find(|(n, _)| n == "position"))
        .map(|(_, v)| v.clone());
    assert_eq!(
        pos,
        Some(ReflectValue::Vec3(glam::Vec3::new(9.0, 9.0, 9.0)))
    );

    done.store(true, Ordering::Relaxed);
    main_loop.join().unwrap();
}

/// Play is a wire toggle in remote mode: the project runs its own systems in place, and the editor
/// records that it is playing so the toolbar and the refresh cadence follow.
#[test]
fn play_toggles_the_remote_gate() {
    let server = RemoteServer::start(&test_socket_name()).expect("bind");
    let socket = server.name().to_owned();
    let done = Arc::new(AtomicBool::new(false));
    let loop_done = Arc::clone(&done);
    let main_loop = std::thread::spawn(move || {
        let mut res = ecs();
        while !loop_done.load(Ordering::Relaxed) {
            for item in server.take_pending() {
                let resp = handle(&item.request, &mut res);
                let _ = item.reply.send(resp);
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        kooch_core::run_state::Playing::is_playing(&res)
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
    assert!(!state.playing, "starts paused");
    editor.insert(state);

    assert!(dispatch(&mut editor, &EditorAction::Play));
    assert!(
        editor.get::<RemoteState>().unwrap().playing,
        "editor did not record the play state"
    );

    done.store(true, Ordering::Relaxed);
    assert!(main_loop.join().unwrap(), "project did not start playing");
}

/// A non-ECS action is not owned by the remote sink.
#[test]
fn non_ecs_action_falls_through() {
    let mut editor = ecs();
    assert!(!dispatch(&mut editor, &EditorAction::CloseProject));
}

/// Saving one scene is the project's business, and carries which scene.
#[test]
fn saving_one_scene_names_it() {
    let editor = ecs();
    let id = kooch_core::Guid::new_v4();

    let named = |action| match super::classify(&action, &editor) {
        Some(super::Edit::SaveOneScene { scene, as_new }) => (scene, as_new),
        _ => panic!("a per-scene save did not classify as one"),
    };

    assert_eq!(
        named(EditorAction::SaveOpenScene(id)),
        (id, false),
        "Save lost the scene, or asked for a path instead of using the file",
    );
    assert_eq!(
        named(EditorAction::SaveOpenSceneAs(id)),
        (id, true),
        "Save As lost the scene, or wrote over the existing file without asking",
    );
}

/// A world edit that `classify` refuses must be routed before it, or it falls through to
/// `apply_non_ecs_action` — which does not know it either — and the menu entry does nothing, in
/// silence.
#[test]
fn every_unclassified_world_edit_is_routed() {
    use crate::actions::{EditorAction, SpawnTarget};

    let mut resources = kooch_core::resource::Resources::new();
    let unroutable: Vec<&str> = [
        (
            "SpawnMesh",
            EditorAction::SpawnMesh {
                path: std::path::PathBuf::from("meshes/primitives/cube.glb"),
                name: "Cube".to_owned(),
            },
        ),
        (
            "SpawnBlock",
            EditorAction::SpawnBlock {
                into: SpawnTarget::Active,
                shape: kooch_blockmesh::Shape::DEFAULTS[0],
            },
        ),
        (
            "BlockEdit",
            EditorAction::BlockEdit {
                entity: kooch_ecs::Entity::new(0, 0),
                source: kooch_core::Guid::new_v4(),
                before: Box::default(),
                after: Box::default(),
            },
        ),
    ]
    .into_iter()
    .filter(|(_, action)| {
        assert!(
            action.is_a_world_edit(),
            "the premise: these are world edits"
        );
        super::classify(action, &mut resources).is_none()
    })
    .map(|(name, _)| name)
    .collect();

    assert_eq!(
        unroutable,
        ["SpawnMesh", "SpawnBlock", "BlockEdit"],
        "these two are refused by `classify` and must therefore be \
         handled in `dispatch` before it — check that they still are",
    );
}

/// 🔴 A block spawns two ways — locally and over the wire — and the two lists had already drifted
/// twice.
#[test]
fn a_blocks_components_agree_by_id_and_name() {
    for (type_id, name) in kooch_blockmesh::block_components() {
        let by_name: Option<std::any::TypeId> = [
            (
                std::any::type_name::<kooch_blockmesh::Block>(),
                std::any::TypeId::of::<kooch_blockmesh::Block>(),
            ),
            (
                std::any::type_name::<kooch_ecs::mesh_renderer::MeshRenderer>(),
                std::any::TypeId::of::<kooch_ecs::mesh_renderer::MeshRenderer>(),
            ),
            (
                std::any::type_name::<kooch_physics::components::Collider>(),
                std::any::TypeId::of::<kooch_physics::components::Collider>(),
            ),
            (
                std::any::type_name::<kooch_physics::components::PhysicsBody>(),
                std::any::TypeId::of::<kooch_physics::components::PhysicsBody>(),
            ),
        ]
        .into_iter()
        .find(|(candidate, _)| *candidate == name)
        .map(|(_, id)| id);

        assert_eq!(by_name, Some(type_id), "{name} is paired with another type");
    }
}

/// A block collides, so its list carries what physics actually walks.
#[test]
fn a_block_is_a_body() {
    let names: Vec<&str> = kooch_blockmesh::block_components()
        .iter()
        .map(|(_, name)| *name)
        .collect();
    assert!(
        names.contains(&std::any::type_name::<kooch_physics::components::PhysicsBody>()),
        "physics walks bodies, not colliders: {names:?}",
    );
}
