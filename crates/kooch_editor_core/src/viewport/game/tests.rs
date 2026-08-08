use super::*;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::query::AccessTracker;

/// Two cameras: the editor's at the priority it really ships with,
/// and a gameplay one below it. The View panel picks the editor's;
/// this panel must not.
fn world_with_both_cameras(editor_priority: i32, game_priority: i32) -> Resources {
    let mut r = Resources::new();
    let mut alloc = EntityAllocator::new();
    let editor_cam = alloc.spawn();
    let game_cam = alloc.spawn();
    r.insert(alloc);

    let mut registry = ComponentRegistry::new();
    registry.register_cpu_reflected::<PerspectiveCamera>();
    registry.register_cpu_reflected::<GlobalTransform>();
    registry.register_cpu::<EditorCamera>();

    let mut archetypes = ArchetypeRegistry::new();
    let editor_sig = [
        std::any::TypeId::of::<PerspectiveCamera>(),
        std::any::TypeId::of::<GlobalTransform>(),
        std::any::TypeId::of::<EditorCamera>(),
    ]
    .into_iter()
    .collect();
    let game_sig = [
        std::any::TypeId::of::<PerspectiveCamera>(),
        std::any::TypeId::of::<GlobalTransform>(),
    ]
    .into_iter()
    .collect();
    let editor_arch = archetypes.get_or_create(editor_sig);
    let game_arch = archetypes.get_or_create(game_sig);

    for (entity, priority, x) in [
        (editor_cam, editor_priority, 10.0),
        (game_cam, game_priority, -7.0),
    ] {
        registry
            .get_cpu_mut::<PerspectiveCamera>()
            .expect("registered")
            .insert(
                entity,
                PerspectiveCamera {
                    priority,
                    active: true,
                    ..Default::default()
                },
            );
        // Distinct positions so the assert can tell which camera the
        // matrices came from.
        registry
            .get_cpu_mut::<GlobalTransform>()
            .expect("registered")
            .insert(
                entity,
                GlobalTransform {
                    matrix: glam::Mat4::from_translation(glam::Vec3::new(x, 0.0, 0.0)),
                },
            );
    }
    registry
        .get_cpu_mut::<EditorCamera>()
        .expect("registered")
        .insert(editor_cam, EditorCamera);
    archetypes.register_entity(editor_cam, editor_arch);
    archetypes.register_entity(game_cam, game_arch);

    r.insert(registry);
    r.insert(archetypes);
    r.insert(AccessTracker::new());
    r
}

#[test]
fn the_editor_camera_is_never_the_game_camera() {
    // 1000 is EDITOR_CAMERA_PRIORITY: it outranks everything in the
    // View panel by design, which is exactly why picking "highest
    // priority" here would show the authoring camera.
    let r = world_with_both_cameras(1000, 0);
    let cam_pos = gameplay_camera(&r)
        .expect("a gameplay camera exists")
        .position();
    assert_eq!(cam_pos.x, -7.0, "picked the editor camera");
}

#[test]
fn a_gameplay_camera_at_the_editors_priority_still_wins() {
    // The identity test is the marker, not the number. A game that
    // authors a camera at 1000 for its own reasons must not make the
    // Game panel show the editor's view.
    let r = world_with_both_cameras(1000, 1000);
    let cam_pos = gameplay_camera(&r)
        .expect("a gameplay camera exists")
        .position();
    assert_eq!(cam_pos.x, -7.0);
}

#[test]
fn no_gameplay_camera_reports_none() {
    // Not "black": the panel needs to distinguish "nothing to show"
    // from "the game renders black", and says which component to add.
    let mut r = world_with_both_cameras(1000, 0);
    {
        let registry = r.get_mut::<ComponentRegistry>().expect("registered");
        let cams = registry
            .get_cpu_mut::<PerspectiveCamera>()
            .expect("registered");
        for (_, cam) in cams.iter_mut() {
            cam.active = false;
        }
    }
    assert!(gameplay_camera(&r).is_none());
}
