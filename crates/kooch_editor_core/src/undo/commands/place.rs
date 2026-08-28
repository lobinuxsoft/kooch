//! Where a newly built entity goes: which scene it belongs to, and what
//! it hangs off.
//!
//! 🔴 Shared rather than owned by [`super::spawn`], because an entity
//! that belongs to no scene is not a spawn-only failure. It shows up
//! under "Unsaved" and is adopted by whichever scene happened to be
//! active at the next save — which is not what a menu opened on a
//! different one asked for, whether the entity came from a Spawn or from
//! a Paste.

use std::any::TypeId;

use kooch_core::resource::Resources;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;

use crate::actions::SpawnTarget;

/// The scene a target names, creating one if that is what it asks for.
///
/// 🔴 Call this ONCE per gesture, not once per entity. `NewScene` makes
/// a scene every time it is asked, so resolving it inside a loop over a
/// clipboard of five entities produces five scenes holding one entity
/// each — which is the shape of a paste nobody wanted.
///
/// [`SpawnTarget::ChildOf`] is absent on purpose: reparenting is an edit
/// to the entity, not a lookup, so it stays with the caller that has one
/// to reparent. Use [`scene_of`] on the parent afterwards.
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
