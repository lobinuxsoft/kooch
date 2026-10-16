//! Keeping a swept block's geometry in step with the curve it is swept along (#1152).
//!
//! 🔴 **The sweep completes its own entity.** What the author adds is a component, not a block, so
//! there is no spawn command to write the `.block` file the way `SpawnBlockCommand` does. The first
//! time a curve has something to sweep, this writes the asset and gives the entity the block
//! components to point at it — after that it is an ordinary block, with ordinary faces, colliders
//! and extrudes.
//!
//! Baked to the asset rather than rebuilt at runtime, the opposite of #1428's placement: a
//! placement writes transforms and a sweep writes geometry, and geometry belongs in a file the
//! game loads.

use std::any::TypeId;
use std::collections::{HashMap, HashSet};

use kooch_blockmesh::{Block, SplineSweep, sweep};
use kooch_core::resource::Resources;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;
use kooch_ecs::query::Query;
use kooch_ecs::spline::{Spline, curve_revision};

/// What each sweep was last built from, as the curve's own revision mixed with its parameters.
#[derive(Default)]
pub(crate) struct BuiltSweeps {
    revisions: HashMap<Entity, u64>,
    /// Rebuilt but not written yet. 🔴 Dragging a knot changes the curve every frame, and writing
    /// the `.block` on each of them is a file rewritten sixty times a second for one gesture. The
    /// mesh still updates live; only the write waits for the hand to come off.
    unsaved: HashSet<Entity>,
}

/// Rebuilds every sweep whose curve or parameters changed since it was last seen.
pub(crate) fn sync_sweeps(resources: &mut Resources) {
    // Taken out first: the query below borrows the world, and the record of what was built has to
    // be readable while it runs.
    let mut built = resources.remove::<BuiltSweeps>().unwrap_or_default();
    let mut seen = HashSet::new();
    let mut changed = Vec::new();

    Query::<(&Spline, &SplineSweep)>::new(resources).for_each_entity(|entity, (spline, of)| {
        seen.insert(entity);
        let revision = revision(spline, of);
        if built.revisions.insert(entity, revision) == Some(revision) {
            return;
        }
        changed.push((entity, sweep(&spline.points, spline.closed, of)));
    });
    // A sweep whose component or entity is gone leaves nothing behind to rebuild.
    built.revisions.retain(|entity, _| seen.contains(entity));
    built.unsaved.retain(|entity| seen.contains(entity));

    for (entity, mesh) in changed {
        let Some(source) = source_of(resources, entity) else {
            continue;
        };
        if super::set_shape(resources, source, &mesh) {
            super::announce(resources, source);
            built.unsaved.insert(entity);
        }
    }

    // The hand is still on it: the file waits, the viewport does not.
    if !built.unsaved.is_empty() && !dragging(resources) {
        for entity in std::mem::take(&mut built.unsaved) {
            super::save(resources, entity);
        }
    }
    resources.insert(built);
}

/// Whether the author is mid-gesture on a curve, by a grip or by the transform gizmo.
fn dragging(resources: &Resources) -> bool {
    crate::gizmos::spline_handles::is_dragging(resources)
        || resources
            .get::<kooch_gizmos_handles::HandleSet>()
            .is_some_and(kooch_gizmos_handles::HandleSet::is_active)
}

/// The `.block` this entity's sweep writes to, created along with the components that point at it
/// the first time it is needed.
///
/// ⚠️ Not undoable, unlike a spawned block. Undoing the component that asked for the sweep leaves
/// the asset on disk — a stray file in `assets/blocks`, which is recoverable, where rolling an
/// asset back from a system that cannot see the command stack is not.
fn source_of(resources: &mut Resources, entity: Entity) -> Option<kooch_core::Guid> {
    if let Some(source) = resources
        .get::<ComponentRegistry>()
        .and_then(|registry| registry.get_cpu::<Block>())
        .and_then(|storage| storage.get(entity))
        .and_then(|block| block.source)
    {
        return Some(source);
    }

    // Written empty: the mesh the caller just built lands through `set_shape` on the same frame,
    // and writing it twice would put the curve's geometry in two places that can disagree.
    let (_, guid) = crate::actions::asset_ops::new_block_asset_from(
        resources,
        "Sweep",
        &kooch_blockmesh::BlockMesh::default(),
    )?;
    for (type_id, _) in kooch_blockmesh::block_components() {
        add_component(resources, entity, type_id);
    }
    // The prototype grid rather than the white default, so a sweep reads the way a block does.
    if let Some(material) = crate::undo::prototype_material(resources)
        && let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<kooch_ecs::mesh_renderer::MeshRenderer>()
        && let Some(renderer) = storage.get_mut(entity)
    {
        renderer.material = Some(material);
    }
    let registry = resources.get_mut::<ComponentRegistry>()?;
    let block = registry.get_cpu_mut::<Block>()?.get_mut(entity)?;
    block.source = Some(guid);
    Some(guid)
}

/// Gives `entity` a default-built component, and tells the archetypes it has one.
fn add_component(resources: &mut Resources, entity: Entity, type_id: TypeId) {
    let inserted = resources
        .get_mut::<ComponentRegistry>()
        .is_some_and(|registry| registry.insert_default_reflected(&type_id, entity));
    if inserted
        && let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>()
        && let Some(current) = archetypes.entity_archetype(entity)
    {
        let after = archetypes.archetype_after_add_dynamic(current, type_id);
        archetypes.register_entity(entity, after);
    }
}

/// What the sweep was built from, as one number: the curve, and the profile taken through it.
fn revision(spline: &Spline, of: &SplineSweep) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    curve_revision(&spline.points, spline.closed).hash(&mut hasher);
    of.profile.hash(&mut hasher);
    of.sides.hash(&mut hasher);
    of.spans.hash(&mut hasher);
    of.caps.hash(&mut hasher);
    hasher.finish()
}
