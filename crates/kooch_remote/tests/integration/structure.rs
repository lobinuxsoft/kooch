//! Moving entities and scene membership, and notifications that do not wait.

use super::*;

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
            notify: false,
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

/// Membership travels once, in `scene`, never among components. 🔴 As a reflected component the skip
/// list must stop it, or instance guids file entities wrongly.
#[test]
fn membership_travels_beside_the_components_not_among_them() {
    let mut resources = ecs();
    let mut manager = kooch_ecs::SceneManager::new();
    let active = manager.active_id().expect("a scene");
    resources.insert(manager);

    let entity = match call(
        &mut resources,
        Method::Spawn {
            name: Some("Rig".into()),
            scene: None,
            parent: None,
        },
    ) {
        ResponseData::Spawned { entity } => entity,
        other => panic!("spawn: {other:?}"),
    };

    let entities = match call(&mut resources, Method::ListEntities { since: None }) {
        ResponseData::Entities { entities, .. } => entities,
        other => panic!("list: {other:?}"),
    };
    let mirrored = entities
        .iter()
        .find(|e| e.id == entity)
        .expect("the spawned entity");

    assert_eq!(
        mirrored.scene,
        Some(active),
        "membership did not travel at all",
    );
    let named: Vec<&str> = mirrored
        .components
        .iter()
        .map(|c| c.type_name.as_str())
        .collect();
    assert!(
        !named.iter().any(|name| name.contains("SceneMember")),
        "membership travelled twice: {named:?}",
    );
}

/// 🔴 `notify` must not wait for the host — `call` cost 5.9 ms a frame for a discarded reply. The
/// server never drains here, so returning promptly is the assertion.
#[test]
fn notify_does_not_wait_for_the_host() {
    let server = RemoteServer::start(&test_socket_name()).expect("bind a port");
    let client = RemoteClient::new(server.name());

    let started = std::time::Instant::now();
    client
        .notify(kooch_remote::protocol::Method::Ping)
        .expect("the socket accepted the notification");
    let elapsed = started.elapsed();

    assert!(
        elapsed < std::time::Duration::from_millis(200),
        "notify waited {elapsed:?} for a host that never answers",
    );
    // And it really did arrive — a `notify` that dropped the request on
    // the floor would also return fast.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let mut arrived = false;
    while std::time::Instant::now() < deadline && !arrived {
        arrived = !server.take_pending().is_empty();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(arrived, "the host never saw the notification");
}
