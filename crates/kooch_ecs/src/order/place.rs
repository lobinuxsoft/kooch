//! Moving an entity to a place among its siblings.
//!
//! The policy lives here rather than in the editor so both the local path
//! and the remote host reach the same one. The editor says *where* — this
//! parent, before that sibling — and the numbering is decided once.

use kooch_core::resource::Resources;

use crate::component::ComponentRegistry;
use crate::entity::Entity;
use crate::hierarchy::{Parent, reparent};

use super::Order;

/// Moves `entity` under `parent`, immediately before `before`.
///
/// `parent: None` makes it a root of its scene; `before: None` puts it
/// last. Returns `false` when the move is refused — moving an entity into
/// its own subtree, which would detach that subtree from the world.
///
/// # Why the caller does not pass a number
///
/// "Before that one" is what a drag means, and a number is one answer to
/// it. Letting the caller pick would put the renumbering rule in every
/// caller, and they would disagree the first time a gap ran out.
pub fn place(
    resources: &mut Resources,
    entity: Entity,
    parent: Option<Entity>,
    before: Option<Entity>,
) -> bool {
    if let Some(parent) = parent
        && (parent == entity || is_descendant(resources, parent, entity))
    {
        return false;
    }

    reparent(resources, entity, parent);

    // Read *after* the reparent, so `entity` is already among them and
    // the list is the one being ordered rather than the one before it.
    let mut siblings = siblings_of(resources, entity, parent);
    siblings.retain(|&e| e != entity);
    let at = match before {
        Some(before) => siblings.iter().position(|&e| e == before),
        None => None,
    }
    .unwrap_or(siblings.len());
    siblings.insert(at, entity);

    assign(resources, &siblings, at);
    true
}

/// Gives `siblings` order values, writing as few as it can.
///
/// The one at `moved` is the entity that just arrived; everything else is
/// already in the order the user sees. One value is written when there is
/// room between its new neighbours — which is what keeps moving one of
/// thirty-six instances from showing up as thirty-six changed fields in
/// the scene diff.
///
/// The whole group is renumbered when there is not: either nobody has
/// been ordered yet (a scene authored before this existed), or repeated
/// drops in one place have used the gap up.
fn assign(resources: &mut Resources, siblings: &[Entity], moved: usize) {
    let values: Vec<Option<u32>> = siblings.iter().map(|&e| order_of(resources, e)).collect();

    // Every neighbour has to have a value for "between" to mean anything.
    let neighbours_known = values
        .iter()
        .enumerate()
        .all(|(i, v)| i == moved || v.is_some());
    let room = neighbours_known
        .then(|| {
            Order::between(
                moved.checked_sub(1).and_then(|i| values[i]),
                values.get(moved + 1).copied().flatten(),
            )
        })
        .flatten();

    match room {
        Some(value) => set_order(resources, siblings[moved], value),
        None => {
            for (entity, value) in siblings.iter().zip(Order::spaced(siblings.len())) {
                set_order(resources, *entity, value);
            }
        }
    }
}

/// The entities that share `entity`'s parent, in their current order.
///
/// 🔴 Found by scanning `Parent`, not by reading `Children`. `Children`
/// is built when a scene loads and **`reparent` does not maintain it** —
/// which is why the World panel builds the hierarchy from `Parent` too.
/// Read from `Children`, a child moved at runtime is invisible to its own
/// parent, and every sibling in the group ends up with the same order
/// value.
fn siblings_of(resources: &Resources, entity: Entity, parent: Option<Entity>) -> Vec<Entity> {
    use crate::archetype_registry::ArchetypeRegistry;
    use crate::scene_member::SceneMember;

    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return vec![entity];
    };
    let Some(archetypes) = resources.get::<ArchetypeRegistry>() else {
        return vec![entity];
    };
    let parents = registry.get_cpu::<Parent>();
    let members = registry.get_cpu::<SceneMember>();
    // Roots of *this* scene only. Without the filter, two open scenes'
    // roots would be numbered as one group and interleave.
    let scene = members.and_then(|s| s.get(entity)).map(|m| m.scene);

    let mut siblings: Vec<Entity> = archetypes
        .iter_matching(&[])
        .flat_map(|archetype| archetype.entities().iter().copied())
        .filter(|e| parents.and_then(|s| s.get(*e)).map(|p| p.entity) == parent)
        .filter(|e| {
            // A parent already scopes the group; a scene has to be asked
            // for.
            parent.is_some() || members.and_then(|s| s.get(*e)).map(|m| m.scene) == scene
        })
        .collect();
    if !siblings.contains(&entity) {
        siblings.push(entity);
    }
    // The order they are shown in, which is what "before that one" is
    // measured against — the same key the World panel sorts by.
    let orders = registry.get_cpu::<Order>();
    siblings.sort_by_key(|e| {
        let value = orders.and_then(|s| s.get(*e)).map(|o| o.value);
        (value.is_none(), value.unwrap_or(0), e.index())
    });
    siblings
}

pub(crate) fn order_of(resources: &Resources, entity: Entity) -> Option<u32> {
    resources
        .get::<ComponentRegistry>()?
        .get_cpu::<Order>()?
        .get(entity)
        .map(|o| o.value)
}

/// Writes one entity's order, adding the component and moving it to the
/// archetype that now describes it.
fn set_order(resources: &mut Resources, entity: Entity, value: u32) {
    let grew = match resources.get_mut::<ComponentRegistry>() {
        Some(registry) => {
            registry.register_cpu_reflected::<Order>();
            let storage = registry.get_cpu_mut::<Order>();
            match storage {
                Some(storage) => storage.insert(entity, Order::new(value)).is_none(),
                None => false,
            }
        }
        None => false,
    };
    if !grew {
        return;
    }
    if let Some(archetypes) = resources.get_mut::<crate::archetype_registry::ArchetypeRegistry>()
        && let Some(current) = archetypes.entity_archetype(entity)
    {
        let next = archetypes.archetype_after_add_dynamic(current, std::any::TypeId::of::<Order>());
        archetypes.register_entity(entity, next);
    }
}

/// Whether `candidate` is inside `root`'s subtree.
///
/// Walks up from `candidate`, which costs the depth of the tree rather
/// than the size of the subtree.
fn is_descendant(resources: &Resources, candidate: Entity, root: Entity) -> bool {
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return false;
    };
    let Some(parents) = registry.get_cpu::<Parent>() else {
        return false;
    };
    let mut at = parents.get(candidate).map(|p| p.entity);
    // Bounded so a cycle already in the world cannot hang the editor.
    for _ in 0..1024 {
        match at {
            Some(e) if e == root => return true,
            Some(e) => at = parents.get(e).map(|p| p.entity),
            None => return false,
        }
    }
    true
}
