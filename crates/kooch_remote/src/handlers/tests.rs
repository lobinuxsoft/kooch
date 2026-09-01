use super::*;
use kooch_ecs::hierarchy::{Children, Parent};

fn world() -> Resources {
    let mut resources = Resources::new();
    resources.insert(kooch_ecs::allocator::EntityAllocator::new());
    resources.insert(ComponentRegistry::new());
    resources.insert(kooch_ecs::archetype_registry::ArchetypeRegistry::new());
    resources.insert(kooch_ecs::query::AccessTracker::new());
    resources.insert(Commands::new());
    resources
}

fn spawn(resources: &mut Resources) -> kooch_ecs::entity::Entity {
    let mut commands = resources.remove::<Commands>().unwrap();
    let entity = commands.spawn(resources).id();
    commands.apply(resources);
    resources.insert(commands);
    entity
}

fn attach(
    resources: &mut Resources,
    parent: kooch_ecs::entity::Entity,
    child: kooch_ecs::entity::Entity,
) {
    let registry = resources.get_mut::<ComponentRegistry>().unwrap();
    registry.register_cpu_reflected::<Parent>();
    registry.register_cpu_reflected::<Children>();
    if let Some(storage) = registry.get_cpu_mut::<Parent>() {
        storage.insert(child, Parent { entity: parent });
    }
    if let Some(storage) = registry.get_cpu_mut::<Children>() {
        let existing = storage.get(parent).map(|c| c.entities.clone());
        let mut entities = existing.unwrap_or_default();
        entities.push(child);
        storage.insert(parent, Children { entities });
    }
}

fn alive(resources: &Resources, entity: kooch_ecs::entity::Entity) -> bool {
    resources
        .get::<EntityAllocator>()
        .is_some_and(|a| a.is_alive(entity))
}

/// Despawning a parent has to take its whole subtree. A child left
/// behind holds a `Parent` pointing at a dead handle: nothing in the
/// hierarchy can reach it, its transform derives from an entity that
/// no longer exists, and it survives into the saved scene.
#[test]
fn despawning_a_parent_takes_its_descendants() {
    let mut resources = world();
    let root = spawn(&mut resources);
    let child = spawn(&mut resources);
    let grandchild = spawn(&mut resources);
    attach(&mut resources, root, child);
    attach(&mut resources, child, grandchild);

    despawn(&mut resources, EntityId::from(root)).unwrap();

    assert!(!alive(&resources, root));
    assert!(!alive(&resources, child), "the child outlived its parent");
    assert!(
        !alive(&resources, grandchild),
        "a deeper descendant outlived the subtree",
    );
}

/// A sibling is not a descendant. Over-collecting would silently
/// delete half the scene.
#[test]
fn despawning_leaves_everything_outside_the_subtree_alone() {
    let mut resources = world();
    let root = spawn(&mut resources);
    let child = spawn(&mut resources);
    let bystander = spawn(&mut resources);
    attach(&mut resources, root, child);

    despawn(&mut resources, EntityId::from(root)).unwrap();

    assert!(!alive(&resources, child));
    assert!(
        alive(&resources, bystander),
        "an unrelated entity was taken"
    );
}

/// Stop must be indistinguishable from never having pressed play.
///
/// The snapshot is taken on the way in and put back on the way out, so a
/// value a system moved during play returns to what was authored.
#[test]
fn stop_puts_an_authored_value_back() {
    use kooch_ecs::transform::Transform;

    let mut resources = world();
    resources
        .get_mut::<ComponentRegistry>()
        .unwrap()
        .register_cpu_reflected::<Transform>();
    let entity = spawn(&mut resources);
    {
        let registry = resources.get_mut::<ComponentRegistry>().unwrap();
        registry.insert_default_reflected(&std::any::TypeId::of::<Transform>(), entity);
        let storage = registry.get_cpu_mut::<Transform>().unwrap();
        let mut authored = Transform::default();
        authored.position = glam::Vec3::new(1.0, 2.0, 3.0);
        storage.insert(entity, authored);
    }
    let arch = resources.get_mut::<ArchetypeRegistry>().unwrap();
    let empty = arch.get_or_create(Default::default());
    arch.register_entity(entity, empty);
    let next = arch.archetype_after_add::<Transform>(empty);
    arch.register_entity(entity, next);

    set_playing(&mut resources, true).expect("play");

    // What a gameplay system would do.
    resources
        .get_mut::<ComponentRegistry>()
        .unwrap()
        .get_cpu_mut::<Transform>()
        .unwrap()
        .get_mut(entity)
        .unwrap()
        .position = glam::Vec3::new(99.0, 99.0, 99.0);

    set_playing(&mut resources, false).expect("stop");

    let position = resources
        .get::<ComponentRegistry>()
        .unwrap()
        .get_cpu::<Transform>()
        .unwrap()
        .get(entity)
        .expect("the entity survived the restore")
        .position;
    assert_eq!(
        position,
        glam::Vec3::new(1.0, 2.0, 3.0),
        "stop left the world where play moved it",
    );
}

/// Helper: the entity ids and `full` flag of an `Entities` reply.
fn entities_reply(response: Response) -> (Vec<EntityId>, bool) {
    match response.payload {
        crate::protocol::ResponsePayload::Result(ResponseData::Entities {
            entities, full, ..
        }) => (entities.iter().map(|e| e.id).collect(), full),
        other => panic!("expected an Entities reply, got {other:?}"),
    }
}

/// Stop restores the world the last full pull already described, so the
/// diff comes out empty — and the editor, which learned the played
/// positions from the *moved* pull, keeps drawing them.
///
/// Two caches describe one world and nothing reconciles them:
/// `SnapshotCache` never sees a play session, `MovedCache` is the only
/// thing that does.
#[test]
fn stop_tells_the_caller_the_world_moved_back() {
    use kooch_ecs::transform::Transform;

    let mut resources = world();
    resources
        .get_mut::<ComponentRegistry>()
        .unwrap()
        .register_cpu_reflected::<Transform>();
    let entity = spawn(&mut resources);
    {
        let registry = resources.get_mut::<ComponentRegistry>().unwrap();
        registry.insert_default_reflected(&std::any::TypeId::of::<Transform>(), entity);
    }
    let arch = resources.get_mut::<ArchetypeRegistry>().unwrap();
    let empty = arch.get_or_create(Default::default());
    arch.register_entity(entity, empty);
    let next = arch.archetype_after_add::<Transform>(empty);
    arch.register_entity(entity, next);

    let move_to = |resources: &mut Resources, x: f32| {
        resources
            .get_mut::<ComponentRegistry>()
            .unwrap()
            .get_cpu_mut::<Transform>()
            .unwrap()
            .get_mut(entity)
            .unwrap()
            .position = glam::Vec3::new(x, 0.0, 0.0);
    };

    // The editor's pull while authoring.
    move_to(&mut resources, 1.0);
    let (_, full) = entities_reply(list_entities(1, &mut resources, None));
    assert!(full, "the first pull is a full one");
    let held = 1u64;

    set_playing(&mut resources, true).expect("play");
    // A gameplay system moves it, and the editor learns that through the
    // cheap pull — the only one it makes while playing.
    move_to(&mut resources, 9.0);
    let _ = list_moved(2, &mut resources, None);

    set_playing(&mut resources, false).expect("stop");

    let (changed, full) = entities_reply(list_entities(3, &mut resources, Some(held)));
    assert!(
        full || !changed.is_empty(),
        "stop said nothing, so the editor is still drawing where play left it",
    );
}

// -- The systems panel's wire (#982 step 3) -----------------------------

fn catalog_of(names: &[(&str, kooch_core::schedule::SystemSource)]) -> Resources {
    use kooch_core::schedule::{SystemCatalog, SystemKey, SystemRecord};

    let mut resources = world();
    resources.insert(SystemCatalog::new(
        names
            .iter()
            .map(|(name, source)| SystemRecord {
                stage: kooch_core::stage::Stage::Update,
                name: (*name).to_owned(),
                key: SystemKey::new(*name),
                source: *source,
                gpu: false,
            })
            .collect(),
    ));
    resources
}

/// A host that has not published yet is not a host with no systems, but
/// the panel has nothing to draw either way — it must not panic.
#[test]
fn an_unpublished_host_lists_nothing() {
    let resources = world();
    assert!(list_systems(&resources).is_empty());
}

#[test]
fn the_list_says_which_half_scheduled_each() {
    use kooch_core::schedule::SystemSource;

    let resources = catalog_of(&[
        ("kooch_render::upload", SystemSource::Engine),
        ("game::systems::jump", SystemSource::Project),
    ]);

    let listed = list_systems(&resources);
    assert_eq!(listed.len(), 2);
    assert!(!listed[0].project);
    assert!(listed[1].project);
    assert_eq!(listed[1].short, "jump", "the panel shows the short name");
    assert!(listed.iter().all(|system| system.enabled));
}

/// The round trip the panel makes: switch one off, and the next list
/// says so. Read back rather than assumed, because that is what the
/// panel does.
#[test]
fn switching_one_off_shows_in_the_next_list() {
    use kooch_core::schedule::SystemSource;

    let mut resources = catalog_of(&[
        ("kooch_render::upload", SystemSource::Engine),
        ("game::systems::jump", SystemSource::Project),
    ]);

    set_system_enabled(&mut resources, "game::systems::jump", 0, false);

    let listed = list_systems(&resources);
    assert!(listed[0].enabled, "the engine one went off too");
    assert!(!listed[1].enabled, "the project one is still running");

    set_system_enabled(&mut resources, "game::systems::jump", 0, true);
    assert!(list_systems(&resources).iter().all(|system| system.enabled));
}

/// A wrapped system is scheduled under the closure's name, so the panel
/// has to address it by the same string the list handed it back.
#[test]
fn the_name_the_list_gives_is_the_name_that_toggles() {
    use kooch_core::schedule::SystemSource;

    let wrapped = "kooch_core::run_state::run_if_playing<game::jump>::{{closure}}";
    let mut resources = catalog_of(&[(wrapped, SystemSource::Project)]);

    let listed = list_systems(&resources);
    set_system_enabled(&mut resources, &listed[0].name, listed[0].nth, false);

    assert!(!list_systems(&resources)[0].enabled);
}
