//! An entity, reduced to what it takes to build it again.
//!
//! # Why one type for three features
//!
//! Copy/paste, undoing a despawn and duplicating an entity are the same
//! question asked three times: *what is on this entity, and how do I put
//! it somewhere else?* Each of them used to answer it on its own —
//! `DuplicateCommand` walks the archetype, `remote_edit::duplicate` walks
//! the registry, and undo captured per-command snapshots — which is three
//! places to forget the same component.
//!
//! So the answer lives here, and it is a plain value: component names and
//! reflected field values, nothing borrowed from the world it came from.
//! That is what lets a clipboard survive the entity being deleted and an
//! undo step survive the mirror refreshing.
//!
//! # What is deliberately not in it
//!
//! **Children.** A capture is one entity. Copying a subtree is a
//! different feature with its own questions (what happens to references
//! *between* the copied entities), and pretending to support it by
//! capturing children without remapping those references would produce
//! copies pointing at the originals.
//!
//! **The editor's own components.** [`MirrorEntity`] marks a row as
//! belonging to the mirror and [`Parent`] is carried separately — sending
//! either to the project is at best a warning in its log and at worst a
//! second entity claiming to be a mirror of the first.

use kooch_core::resource::Resources;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::dynamic_components::DynamicComponents;
use kooch_ecs::entity::Entity;
use kooch_ecs::hierarchy::Parent;
use kooch_ecs::reflect::ReflectValue;

/// One component and every reflected field value it holds.
///
/// Keyed by type **name**, not `TypeId`: the project on the other end of
/// the wire keys components by name, and a component this editor binary
/// has no Rust type for has no `TypeId` here at all.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ComponentState {
    pub name: String,
    pub fields: Vec<(String, ReflectValue)>,
}

/// Everything needed to build one entity from nothing.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct EntityState {
    /// What to call it. Read off the `Name` component, and passed
    /// separately because both spawn paths take the name up front.
    pub name: Option<String>,
    pub components: Vec<ComponentState>,
}

/// The type names never captured, whatever the entity is carrying.
///
/// `Parent` because hierarchy travels as its own field on both sides of
/// the wire, and `MirrorEntity` because it is this editor's bookkeeping —
/// see the module docs.
fn is_editor_only(type_name: &str) -> bool {
    type_name == std::any::type_name::<Parent>()
        || type_name == std::any::type_name::<crate::remote_mirror::MirrorEntity>()
}

/// Reads `entity` out of the world into a value.
///
/// Covers both halves of what an entity can be carrying: components this
/// binary has a type for, read through the reflect registry, and
/// components only the project knows, parked in [`DynamicComponents`].
/// A capture that skipped the parked ones would silently drop exactly the
/// components the user wrote themselves.
pub(crate) fn capture(resources: &Resources, entity: Entity) -> EntityState {
    let mut components = Vec::new();

    if let Some(registry) = resources.get::<ComponentRegistry>() {
        for (type_id, name) in registry.reflected_type_names() {
            if is_editor_only(name) {
                continue;
            }
            let Some(fields) = registry.reflect_get_fields(&type_id, entity) else {
                continue;
            };
            components.push(ComponentState {
                name: name.to_owned(),
                fields,
            });
        }
    }

    if let Some(dynamic) = resources.get::<DynamicComponents>() {
        for (name, fields) in dynamic.iter_entity(entity) {
            if is_editor_only(name) {
                continue;
            }
            components.push(ComponentState {
                name: name.to_owned(),
                fields: fields.to_vec(),
            });
        }
    }

    EntityState {
        name: name_of(&components),
        components,
    }
}

/// Reads one named component off `entity`, or `None` if it has none.
///
/// The narrow half of [`capture`], for an undo step that only has to put
/// a single component back.
pub(crate) fn capture_component(
    resources: &Resources,
    entity: Entity,
    type_name: &str,
) -> Option<ComponentState> {
    let fields = match resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.type_id_by_name(type_name))
    {
        Some(type_id) => resources
            .get::<ComponentRegistry>()
            .and_then(|r| r.reflect_get_fields(&type_id, entity)),
        None => resources.get::<DynamicComponents>().and_then(|d| {
            d.iter_entity(entity)
                .find(|(name, _)| *name == type_name)
                .map(|(_, fields)| fields.to_vec())
        }),
    }?;
    Some(ComponentState {
        name: type_name.to_owned(),
        fields,
    })
}

/// The `Name` component's value, if the capture caught one.
fn name_of(components: &[ComponentState]) -> Option<String> {
    components
        .iter()
        .find(|c| c.name == std::any::type_name::<kooch_ecs::name::Name>())
        .and_then(|c| c.fields.iter().find(|(field, _)| field == "value"))
        .and_then(|(_, value)| match value {
            ReflectValue::String(text) => Some(text.clone()),
            _ => None,
        })
}

/// Names a copy after its source: `Player` → `Player Copy`.
///
/// One suffix, not a counter. Three copies called `Player Copy` are
/// honest about being three copies; `Player Copy 3` claims an ordering
/// the editor does not maintain once any of them is deleted.
pub(crate) fn copy_name(state: &EntityState) -> Option<String> {
    state.name.as_ref().map(|name| format!("{name} Copy"))
}

/// The same entity, named as a copy — in its `Name` component too.
///
/// 🔴 The name is written **twice** when a copy is built: once as the
/// argument to `spawn`, and once again as the captured `Name.value`
/// among the component values. They have to agree, and the captured one
/// lands second. Restoring the source's components verbatim after
/// spawning "Player Copy" writes "Player" back over it — which is what
/// remote Duplicate did until this existed, and why the copies were
/// indistinguishable from their sources in the World panel.
pub(crate) fn as_copy(state: &EntityState) -> EntityState {
    let name = copy_name(state);
    let mut copy = state.clone();
    // 🔴 Membership is not part of what was copied. `capture` takes every
    // reflected component and `SceneMember` is one, so a copy carried the
    // scene it came OUT of — and restoring it wrote that scene straight
    // over wherever the paste had just been placed. Which file a copy
    // lands in is the paste's decision, and only the paste's.
    copy.components
        .retain(|c| c.name != std::any::type_name::<kooch_ecs::SceneMember>());
    copy.name = name.clone();
    let Some(name) = name else {
        return copy;
    };
    for component in &mut copy.components {
        if component.name != std::any::type_name::<kooch_ecs::name::Name>() {
            continue;
        }
        for (field, value) in &mut component.fields {
            if field == "value" {
                *value = ReflectValue::String(name.clone());
            }
        }
    }
    copy
}

/// Writes `state` onto an entity that already exists, in the local world.
///
/// Used by paste and by undoing a despawn: both allocate the entity
/// through the ECS and then need its components put back.
pub(crate) fn restore_local(resources: &mut Resources, entity: Entity, state: &EntityState) {
    for component in &state.components {
        let Some(type_id) = resources
            .get::<ComponentRegistry>()
            .and_then(|r| r.type_id_by_name(&component.name))
        else {
            // A component with no local type is parked, which is where it
            // was read from — the Inspector shows it either way.
            park(resources, entity, component);
            continue;
        };
        let inserted = resources
            .get_mut::<ComponentRegistry>()
            .is_some_and(|r| r.insert_default_reflected(&type_id, entity));
        if inserted {
            advance_archetype(resources, entity, type_id);
        }
        let Some(registry) = resources.get_mut::<ComponentRegistry>() else {
            continue;
        };
        for (field, value) in &component.fields {
            if let Err(e) = registry.reflect_set_field(&type_id, entity, field, value.clone()) {
                tracing::debug!(
                    target: "kooch_editor_core::entity_state",
                    component = %component.name,
                    %field,
                    "field did not restore: {e}",
                );
            }
        }
    }
}

fn park(resources: &mut Resources, entity: Entity, component: &ComponentState) {
    if resources.get::<DynamicComponents>().is_none() {
        resources.insert(DynamicComponents::new());
    }
    if let Some(dynamic) = resources.get_mut::<DynamicComponents>() {
        dynamic.insert(entity, &component.name, component.fields.clone());
    }
}

fn advance_archetype(resources: &mut Resources, entity: Entity, type_id: std::any::TypeId) {
    if let Some(archetypes) =
        resources.get_mut::<kooch_ecs::archetype_registry::ArchetypeRegistry>()
        && let Some(current) = archetypes.entity_archetype(entity)
    {
        let next = archetypes.archetype_after_add_dynamic(current, type_id);
        archetypes.register_entity(entity, next);
    }
}

#[cfg(test)]
mod tests;
