//! An entity, reduced to what it takes to build it again.

use std::collections::HashMap;

use kooch_core::resource::Resources;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::dynamic_components::DynamicComponents;
use kooch_ecs::entity::Entity;
use kooch_ecs::hierarchy::Parent;
use kooch_ecs::reflect::ReflectValue;

/// One component and every reflected field value it holds.
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
fn is_editor_only(type_name: &str) -> bool {
    // `Children` is derived from `Parent` by a system, so a captured one names the ORIGINAL's
    // children: a copy claiming them until the next sync, and a scene saved before it holds a tree
    // with two parents for one child.
    type_name == std::any::type_name::<Parent>()
        || type_name == std::any::type_name::<kooch_ecs::hierarchy::Children>()
        || type_name == std::any::type_name::<crate::remote_mirror::MirrorEntity>()
}

/// Reads `entity` out of the world into a value.
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
pub(crate) fn copy_name(state: &EntityState) -> Option<String> {
    state.name.as_ref().map(|name| format!("{name} Copy"))
}

/// The same entity, named as a copy — in its `Name` component too.
pub(crate) fn as_copy(state: &EntityState) -> EntityState {
    let name = copy_name(state);
    // 🔴 A copy carries what the entity IS, never who it IS nor which scene it belongs to. It DOES
    // keep the prefab it came from: a duplicate that stops following its prefab is an instance the
    // author has to rebuild by hand (#1293).
    let mut copy = without_identity(state);
    copy.components
        .retain(|c| c.name != std::any::type_name::<kooch_ecs::SceneMember>());
    // A member copied without its root belongs to no instance: kept, it would join the original's.
    if !is_instance_root(&copy) {
        copy.components.retain(|c| {
            c.name != std::any::type_name::<kooch_ecs::prefab_instance::PrefabMember>()
        });
    }
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

/// Whether this state is a prefab instance's root, which is what a `PrefabMember` may point at.
pub(crate) fn is_instance_root(state: &EntityState) -> bool {
    let instance = std::any::type_name::<kooch_ecs::prefab_instance::PrefabInstance>();
    state.components.iter().any(|c| c.name == instance)
}

/// Points every copy's `PrefabMember` at the copy of the root it belonged to. A member whose root
/// was not copied belongs to no instance: kept, it would join the original's and take its edits.
pub(crate) fn reroot_prefabs(resources: &mut Resources, copies: &HashMap<Entity, Entity>) {
    use kooch_ecs::prefab_instance::{PrefabInstance, PrefabMember};

    let Some(registry) = resources.get_mut::<ComponentRegistry>() else {
        return;
    };
    let roots: Vec<(Entity, Option<Entity>)> = registry
        .get_cpu::<PrefabMember>()
        .map(|members| {
            copies
                .values()
                .filter_map(|&copy| {
                    let member = members.get(copy)?;
                    let own = registry
                        .get_cpu::<PrefabInstance>()
                        .is_some_and(|instances| instances.get(copy).is_some());
                    // Its own root, the copy of the root it had, or nothing.
                    let root = match own {
                        true => Some(copy),
                        false => copies.get(&member.root).copied(),
                    };
                    Some((copy, root))
                })
                .collect()
        })
        .unwrap_or_default();

    for (copy, root) in roots {
        match root {
            Some(root) => {
                if let Some(members) = registry.get_cpu_mut::<PrefabMember>()
                    && let Some(member) = members.get_mut(copy)
                {
                    member.root = root;
                }
            }
            None => {
                if let Some(members) = registry.get_cpu_mut::<PrefabMember>() {
                    members.remove(copy);
                }
            }
        }
    }
}

/// The same state with the entity's identity left out: a second entity carrying the first's
/// [`PersistentId`](kooch_ecs::PersistentId) IS the first to every reference in the project, and a
/// child of one lands under whichever the loader yields (#1287).
pub(crate) fn without_identity(state: &EntityState) -> EntityState {
    let mut fresh = state.clone();
    let identity = std::any::type_name::<kooch_ecs::PersistentId>();
    fresh.components.retain(|c| c.name != identity);
    fresh
}

/// Writes `state` onto an entity that already exists, in the local world.
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

/// One entity of a captured subtree, and where it sits in it.
#[derive(Clone)]
pub(crate) struct CapturedEntity {
    /// Index of its parent within the capture; `None` for the root.
    pub parent: Option<usize>,
    /// The entity it was taken from, so references into the subtree can be re-pointed at the copy.
    pub source: Entity,
    pub state: EntityState,
}

/// A subtree, root first, each parent before its children.
pub(crate) type CapturedTree = Vec<CapturedEntity>;

/// Reads `root` and everything under it.
pub(crate) fn capture_tree(resources: &Resources, root: Entity) -> CapturedTree {
    let mut tree = Vec::new();
    push_captured(resources, root, None, &mut tree);
    tree
}

fn push_captured(
    resources: &Resources,
    entity: Entity,
    parent: Option<usize>,
    tree: &mut CapturedTree,
) {
    let index = tree.len();
    tree.push(CapturedEntity {
        parent,
        source: entity,
        state: capture(resources, entity),
    });
    for child in children_of(resources, entity) {
        push_captured(resources, child, Some(index), tree);
    }
}

/// This entity's children. Read off `Parent`, which is the authoritative side: `Children` is
/// derived by a system, so a copy made in the same frame as a reparent would miss them.
pub(crate) fn children_of(resources: &Resources, entity: Entity) -> Vec<Entity> {
    let Some(parents) = resources
        .get::<ComponentRegistry>()
        .and_then(|registry| registry.get_cpu::<Parent>())
    else {
        return Vec::new();
    };
    let mut children: Vec<Entity> = parents
        .iter()
        .filter(|(_, parent)| parent.entity == entity)
        .map(|(&child, _)| child)
        .collect();
    // Storage order is a hash; a paste that builds the same tree twice must build it the same way.
    children.sort_by_key(|child| (child.index(), child.generation()));
    children
}

/// The same state with every reference **into the copied subtree** pointing at the copy. A
/// reference out of it is left alone: it names something the copy did not bring with it.
pub(crate) fn remapped(state: &EntityState, copies: &HashMap<Entity, Entity>) -> EntityState {
    use kooch_ecs::reflect::EntityRef;

    let mut state = state.clone();
    for component in &mut state.components {
        for (_, value) in &mut component.fields {
            let ReflectValue::EntityRef(Some(reference)) = value else {
                continue;
            };
            let Some(target) = reference.entity() else {
                continue;
            };
            if let Some(&copy) = copies.get(&target) {
                *value = ReflectValue::EntityRef(Some(EntityRef::live(copy)));
            }
        }
    }
    state
}

/// Builds a captured subtree as copies, under `into`, and answers them in the capture's order.
///
/// 🔴 Two passes: every entity exists before a reference is written, or one pointing at a sibling
/// further down the tree would resolve to nothing purely because of capture order.
pub(crate) fn paste_tree_local(
    resources: &mut Resources,
    tree: &CapturedTree,
    into: Option<Entity>,
) -> Vec<Entity> {
    use kooch_ecs::commands::Commands;

    let mut fresh = Vec::with_capacity(tree.len());
    for _ in tree {
        let Some(mut commands) = resources.remove::<Commands>() else {
            return fresh;
        };
        let entity = commands.spawn(resources).id();
        commands.apply(resources);
        resources.insert(commands);
        fresh.push(entity);
    }

    let copies: HashMap<Entity, Entity> = tree
        .iter()
        .map(|captured| captured.source)
        .zip(fresh.iter().copied())
        .collect();

    for (index, captured) in tree.iter().enumerate() {
        // Only the root is renamed: "Player Copy" with a child called "Head Copy" is not what any
        // editor does, and the name is what an author reads the tree by.
        let state = match captured.parent {
            None => as_copy(&captured.state),
            Some(_) => without_identity(&captured.state),
        };
        restore_local(resources, fresh[index], &remapped(&state, &copies));
        if let Some(name) = captured
            .state
            .name
            .as_deref()
            .filter(|_| captured.parent.is_some())
        {
            rename_local(resources, fresh[index], name);
        }
        let parent = captured.parent.map(|parent| fresh[parent]).or(into);
        kooch_ecs::hierarchy::reparent(resources, fresh[index], parent);
    }
    reroot_prefabs(resources, &copies);
    fresh
}

/// Writes a `Name` onto an entity, for the copies that keep the original's.
fn rename_local(resources: &mut Resources, entity: Entity, name: &str) {
    if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
        let _ = registry.reflect_set_field(
            &std::any::TypeId::of::<kooch_ecs::name::Name>(),
            entity,
            "value",
            ReflectValue::String(name.to_owned()),
        );
    }
}

/// Which entity this one hangs from, if any.
pub(crate) fn parent_of(resources: &Resources, entity: Entity) -> Option<Entity> {
    resources
        .get::<ComponentRegistry>()?
        .get_cpu::<Parent>()?
        .get(entity)
        .map(|parent| parent.entity)
}

/// Which scene an entity belongs to.
pub(crate) fn scene_of(resources: &Resources, entity: Entity) -> Option<kooch_core::Guid> {
    resources
        .get::<ComponentRegistry>()?
        .get_cpu::<kooch_ecs::SceneMember>()?
        .get(entity)
        .map(|member| member.scene)
}

/// The entities of `selection` that no other entity in it contains: a descendant travels with its
/// root, and capturing both would build it twice.
pub(crate) fn roots_of(resources: &Resources, selection: &[Entity]) -> Vec<Entity> {
    selection
        .iter()
        .copied()
        .filter(|&entity| {
            let mut above = parent_of(resources, entity);
            while let Some(parent) = above {
                if selection.contains(&parent) {
                    return false;
                }
                above = parent_of(resources, parent);
            }
            true
        })
        .collect()
}
