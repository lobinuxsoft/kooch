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
        let mesh = sweep(&spline.points, spline.closed, of);
        // A curve of fewer than two knots sweeps nothing, which looks exactly like a system that
        // never ran. Said once per change rather than per frame: the revision was recorded above.
        if mesh.face_count() == 0 {
            tracing::warn!(
                target: "kooch_editor_core::sweep",
                knots = spline.points.len(),
                "a sweep has no curve to follow",
            );
        }
        changed.push((entity, mesh));
    });
    // A sweep whose component or entity is gone leaves nothing behind to rebuild.
    built.revisions.retain(|entity, _| seen.contains(entity));
    built.unsaved.retain(|entity| seen.contains(entity));

    for (entity, mesh) in changed {
        // 🔴 A rebuild that did not land is NOT a built sweep (#1481's lesson, on another system).
        // Recording the revision above is what would make a miss permanent: the curve has not
        // changed, so the guard skips it for ever and the author watches an entity with no mesh.
        match build(resources, entity, &mesh) {
            Some(true) => {
                built.unsaved.insert(entity);
            }
            Some(false) => {}
            None => {
                built.revisions.remove(&entity);
            }
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

/// Puts `mesh` where the entity's block reads it from. `Some(true)` when the loaded asset was
/// replaced and now owes a write, `Some(false)` when the file itself was just written, `None` when
/// it could not land and the sweep has to be tried again next frame.
///
/// 🔴 A brand new sweep writes its geometry **into the file**, rather than writing an empty one and
/// patching the loaded copy. `set_shape` can only replace an asset `sync_blocks` has already built,
/// and a source minted this frame has not been built yet — so the patch silently did nothing, the
/// revision was recorded anyway, and the entity kept an empty mesh for ever. That was the whole bug.
fn build(
    resources: &mut Resources,
    entity: Entity,
    mesh: &kooch_blockmesh::BlockMesh,
) -> Option<bool> {
    if let Some(source) = source_of(resources, entity) {
        return match super::set_shape(resources, source, mesh) {
            true => {
                super::announce(resources, source);
                Some(true)
            }
            // The source exists but its asset is not loaded yet — a scene that has just opened.
            // Retried next frame rather than recorded as built.
            false => None,
        };
    }
    mint(resources, entity, mesh).map(|_| false)
}

/// The source already on the entity's block, if it has one.
fn source_of(resources: &Resources, entity: Entity) -> Option<kooch_core::Guid> {
    resources
        .get::<ComponentRegistry>()?
        .get_cpu::<Block>()?
        .get(entity)?
        .source
}

/// The `.block` this entity's sweep writes to, created along with the components that point at it
/// the first time it is needed.
///
/// ⚠️ Not undoable, unlike a spawned block. Undoing the component that asked for the sweep leaves
/// the asset on disk — a stray file in `assets/blocks`, which is recoverable, where rolling an
/// asset back from a system that cannot see the command stack is not.
fn mint(
    resources: &mut Resources,
    entity: Entity,
    mesh: &kooch_blockmesh::BlockMesh,
) -> Option<kooch_core::Guid> {
    let (path, guid) = crate::actions::asset_ops::new_block_asset_from(resources, "Sweep", mesh)?;
    tracing::info!(
        target: "kooch_editor_core::sweep",
        file = %path.display(),
        faces = mesh.face_count(),
        "a sweep took its own block",
    );
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
