use super::*;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::perspective_camera::PerspectiveCamera;
use kooch_ecs::transform::Transform;

/// An archetype the camera has *left* still lists the marker.
///
/// This is what froze the editor camera: the lookup returned from
/// inside the loop, so the first archetype carrying `EditorCamera`
/// decided the answer even when it held no entities. Adding or
/// removing any component on the camera produces exactly this shape.
#[test]
fn an_empty_archetype_with_the_marker_does_not_hide_the_camera() {
    let mut resources = Resources::new();
    let mut alloc = EntityAllocator::new();
    let camera = alloc.spawn();
    resources.insert(alloc);

    let mut archetypes = ArchetypeRegistry::new();

    // The shell left behind: carries the marker, holds nobody.
    let abandoned: std::collections::BTreeSet<_> = [
        TypeId::of::<EditorCamera>(),
        TypeId::of::<PerspectiveCamera>(),
    ]
    .into_iter()
    .collect();
    archetypes.get_or_create(abandoned);

    // Where the camera actually lives now.
    let current: std::collections::BTreeSet<_> = [
        TypeId::of::<EditorCamera>(),
        TypeId::of::<PerspectiveCamera>(),
        TypeId::of::<Transform>(),
    ]
    .into_iter()
    .collect();
    let current_id = archetypes.get_or_create(current);
    archetypes.register_entity(camera, current_id);
    resources.insert(archetypes);

    assert_eq!(
        find_editor_camera_entity(&resources),
        Some(camera),
        "the lookup stopped at the empty archetype and reported no camera",
    );
}

#[test]
fn no_camera_at_all_is_none() {
    let mut resources = Resources::new();
    resources.insert(EntityAllocator::new());
    resources.insert(ArchetypeRegistry::new());
    assert_eq!(find_editor_camera_entity(&resources), None);
}
