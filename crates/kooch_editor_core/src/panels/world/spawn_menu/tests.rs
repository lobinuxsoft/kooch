//! 🔴 #1399: the components **are** the rig since #1397, so a menu entry is worth exactly whether
//! what it spawns is complete. These build the set each entry names and ask the rig itself.

use super::*;
use kooch_camera::rig::Orphans;
use kooch_core::resource::Resources;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;

/// A world with `spawn` on one entity, as the menu would leave it.
fn spawned(with: &dyn Fn(&mut ComponentRegistry, Entity)) -> Resources {
    let mut resources = Resources::new();
    let mut registry = ComponentRegistry::new();
    let entity = Entity::new(1, 0);
    registry.register_cpu_reflected::<VirtualCamera>();
    registry
        .get_cpu_mut::<VirtualCamera>()
        .unwrap()
        .insert(entity, VirtualCamera::default());
    with(&mut registry, entity);
    resources.insert(registry);
    resources
}

/// Inserts a component of this type with its own default, as the spawn does.
fn add<T: kooch_ecs::component::Component + kooch_ecs::Reflect + Default>(
    registry: &mut ComponentRegistry,
    entity: Entity,
) {
    registry.register_cpu_reflected::<T>();
    registry
        .get_cpu_mut::<T>()
        .unwrap()
        .insert(entity, T::default());
}

/// The whole point: what the menu spawns is a rig the engine has nothing to say about.
#[test]
fn each_rig_spawns_complete() {
    let orbital: &dyn Fn(&mut ComponentRegistry, Entity) = &|r, e| {
        add::<OrbitalFollow>(r, e);
        add::<RotationComposer>(r, e);
        add::<CameraOrbit>(r, e);
    };
    let shoulder: &dyn Fn(&mut ComponentRegistry, Entity) = &|r, e| {
        add::<ThirdPersonFollow>(r, e);
        add::<PanTilt>(r, e);
        add::<CameraOrbit>(r, e);
    };
    for (name, build) in [
        ("Orbital Follow", orbital),
        ("Third Person Follow", shoulder),
    ] {
        let mut resources = spawned(build);
        kooch_camera::rig::report_orphans(&mut resources);
        let said = resources.get::<Orphans>().expect("reported");
        assert!(
            !said.contains(Entity::new(1, 0)),
            "{name} spawns a rig the engine reports on",
        );
    }
}

/// 🔴 And a bare vcam **is** reported, which is why the two entries exist. Without this the test
/// above would pass against a rig that reports nothing because nothing is checked.
#[test]
fn a_bare_vcam_is_reported() {
    let mut resources = spawned(&|_, _| {});
    kooch_camera::rig::report_orphans(&mut resources);
    let said = resources.get::<Orphans>().expect("reported");
    assert!(said.contains(Entity::new(1, 0)), "a bare vcam went unsaid");
}

/// The menu spawns the set these name, so the two cannot drift apart.
#[test]
fn the_menu_spawns_what_the_sets_name() {
    assert_eq!(orbital_rig().len(), 4);
    assert_eq!(shoulder_rig().len(), 4);
    assert!(orbital_rig().contains(&TypeId::of::<OrbitalFollow>()));
    assert!(shoulder_rig().contains(&TypeId::of::<ThirdPersonFollow>()));
    // 🔴 Not a composer: it would re-frame the target and cancel the shoulder on screen (#1379).
    assert!(!shoulder_rig().contains(&TypeId::of::<RotationComposer>()));
}
