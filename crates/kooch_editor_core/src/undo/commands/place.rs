//! Where a newly built entity goes: which scene it belongs to, and what it hangs off.

use std::any::TypeId;

use kooch_core::resource::Resources;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;

use crate::actions::SpawnTarget;

/// The scene a target names, creating one if that is what it asks for.
pub(super) fn resolve_scene(
    resources: &mut Resources,
    into: SpawnTarget,
) -> Option<kooch_core::Guid> {
    match into {
        SpawnTarget::Active => active_scene(resources),
        SpawnTarget::Scene(id) => Some(id),
        // The caller reparents and then asks for the parent's scene: an
        // entity's scene IS its parent's, and authoring a child into
        // another would write it to a file its parent is not in.
        SpawnTarget::ChildOf(parent) => {
            scene_of(resources, parent).or_else(|| active_scene(resources))
        }
        SpawnTarget::NewScene => resources
            .get_mut::<kooch_ecs::SceneManager>()
            .map(|manager| manager.new_scene()),
    }
}

/// The scene new entities land in, if there is one.
pub(super) fn active_scene(resources: &Resources) -> Option<kooch_core::Guid> {
    resources.get::<kooch_ecs::SceneManager>()?.active_id()
}

/// Which scene an entity belongs to.
pub(super) fn scene_of(resources: &Resources, entity: Entity) -> Option<kooch_core::Guid> {
    resources
        .get::<ComponentRegistry>()?
        .get_cpu::<kooch_ecs::SceneMember>()?
        .get(entity)
        .map(|member| member.scene)
}

/// Records which scene the entity belongs to, archetype included, and
/// marks the scene dirty so the panel says it has unsaved work.
pub(super) fn adopt(resources: &mut Resources, entity: Entity, scene: kooch_core::Guid) {
    use kooch_ecs::SceneMember;

    if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
        registry.register_cpu_reflected::<SceneMember>();
        if let Some(storage) = registry.get_cpu_mut::<SceneMember>() {
            storage.insert(entity, SceneMember::new(scene));
        }
    }
    if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>()
        && let Some(current) = archetypes.entity_archetype(entity)
    {
        let next = archetypes.archetype_after_add_dynamic(current, TypeId::of::<SceneMember>());
        archetypes.register_entity(entity, next);
    }
    if let Some(manager) = resources.get_mut::<kooch_ecs::SceneManager>() {
        manager.mark_scene_dirty(scene);
    }
}

/// Takes the membership away, so undoing a move back to "no scene"
/// restores what was there rather than an arbitrary scene.
pub(super) fn disown(resources: &mut Resources, entity: Entity) {
    use kooch_ecs::SceneMember;

    let type_id = TypeId::of::<SceneMember>();
    if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
        registry.remove_component(entity, &type_id);
    }
    if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>()
        && let Some(current) = archetypes.entity_archetype(entity)
    {
        let next = archetypes.archetype_after_remove_dynamic(current, type_id);
        archetypes.register_entity(entity, next);
    }
}

/// Puts a freshly spawned entity where the cursor asked for, in its parent's space.
///
/// 🔴 Resolved ONCE and remembered. A redo re-raycasts nothing: the camera has moved since, and a
/// spawn that answered the cursor's question again would put the entity somewhere the author never
/// pointed (#1459).
///
/// The pivot goes ON the surface. A fresh entity's mesh is not resolved in the same frame for
/// every path — a block's is generated later by `sync_blocks` — and setting down only the ones
/// that happen to be ready would make two menu entries behave differently for no visible reason.
pub(super) fn drop_at(
    resources: &mut Resources,
    entity: Entity,
    at: crate::viewport_pick::DropPoint,
    placed: &mut Option<glam::Vec3>,
) {
    if placed.is_none() {
        *placed = crate::viewport_pick::resolve(resources, at);
    }
    let Some(world) = *placed else {
        return;
    };

    // A child's Transform is in its parent's space, and `place` has already reparented by now.
    let local = match parent_matrix(resources, entity) {
        Some(matrix) => matrix.inverse().transform_point3(world),
        None => world,
    };
    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<kooch_ecs::Transform>()
        && let Some(transform) = storage.get_mut(entity)
    {
        transform.position = local;
    }
}

/// The world matrix of `entity`'s parent, if it has one.
fn parent_matrix(resources: &Resources, entity: Entity) -> Option<glam::Mat4> {
    let registry = resources.get::<ComponentRegistry>()?;
    let parent = registry
        .get_cpu::<kooch_ecs::hierarchy::Parent>()?
        .get(entity)?
        .entity;
    Some(
        registry
            .get_cpu::<kooch_ecs::GlobalTransform>()?
            .get(parent)?
            .matrix,
    )
}
