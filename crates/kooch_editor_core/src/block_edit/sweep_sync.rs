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

use std::collections::{HashMap, HashSet};
use std::path::Path;

use kooch_blockmesh::{Block, SplineSweep, sweep};
use kooch_core::resource::Resources;
use kooch_ecs::component::{ComponentId, ComponentRegistry};
use kooch_ecs::entity::Entity;
use kooch_ecs::query::Query;
use kooch_ecs::spline::{Spline, curve_revision};

use crate::actions::EditorAction;

/// What each sweep was last built from, as the curve's own revision mixed with its parameters.
#[derive(Default)]
pub(crate) struct BuiltSweeps {
    revisions: HashMap<Entity, u64>,
    /// Rebuilt but not written yet. 🔴 Dragging a knot changes the curve every frame, and writing
    /// the `.block` on each of them is a file rewritten sixty times a second for one gesture. The
    /// mesh still updates live; only the write waits for the hand to come off.
    unsaved: HashSet<Entity>,
    /// Sweeps whose block was already minted. 🔴 The components that point at it travel as
    /// actions, so there are frames where the file exists and the entity has not got it yet.
    /// Minting again in one of them would leave a second `.block` behind on every knot the author
    /// drags.
    minted: HashSet<Entity>,
    /// Last reported `(splines, sweeps, both, authoring)`, so the count below is said when it
    /// changes rather than every frame.
    counted: Option<(usize, usize, usize, bool)>,
}

/// Rebuilds every sweep whose curve or parameters changed since it was last seen.
///
/// 🔴 **`project` is passed in, never read from `Resources`.** The render system lifts
/// `ProjectState` out of the table for the whole frame, and this runs inside that frame: asking
/// there answered `None` with a project open, so the sweep had nowhere to write and said so once
/// per frame (#1499).
pub(crate) fn sync_sweeps(
    resources: &mut Resources,
    project: Option<&Path>,
    actions: &mut Vec<EditorAction>,
) {
    // Counted before the gate, so a gated frame does not read like a system that never ran.
    let mut built = resources.remove::<BuiltSweeps>().unwrap_or_default();
    count_sweeps(resources, &mut built, project.is_some());
    let Some(project) = project else {
        resources.insert(built);
        return;
    };
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
    built.minted.retain(|entity| seen.contains(entity));

    for (entity, mesh) in changed {
        // 🔴 A rebuild that did not land is NOT a built sweep (#1481's lesson, on another system).
        // Recording the revision above is what would make a miss permanent: the curve has not
        // changed, so the guard skips it for ever and the author watches an entity with no mesh.
        match build(
            resources,
            project,
            entity,
            &mesh,
            actions,
            &mut built.minted,
        ) {
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

/// What this system can actually see, said when it changes.
///
/// 🔴 A sweep that builds nothing and a sweep the query never found look identical from outside,
/// and that is the difference between a bug in the mesh and a bug in the world. `splines` without
/// `sweeps` means the component did not reach this world; `both` at zero with `sweeps` above it
/// means the two are on different entities; `both` with `authoring` false is the project missing,
/// not the geometry.
fn count_sweeps(resources: &Resources, built: &mut BuiltSweeps, authoring: bool) {
    let mut counts = (0, 0, 0, authoring);
    Query::<&Spline>::new(resources).for_each_entity(|_, _| counts.0 += 1);
    Query::<&SplineSweep>::new(resources).for_each_entity(|_, _| counts.1 += 1);
    Query::<(&Spline, &SplineSweep)>::new(resources).for_each_entity(|_, _| counts.2 += 1);
    if built.counted == Some(counts) {
        return;
    }
    built.counted = Some(counts);
    tracing::info!(
        target: "kooch_editor_core::sweep",
        splines = counts.0,
        sweeps = counts.1,
        both = counts.2,
        authoring,
        "what the sweep sync can see",
    );
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
    project: &Path,
    entity: Entity,
    mesh: &kooch_blockmesh::BlockMesh,
    actions: &mut Vec<EditorAction>,
    minted: &mut HashSet<Entity>,
) -> Option<bool> {
    if let Some(source) = source_of(resources, entity) {
        minted.remove(&entity);
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
    // Already minted, and the components have not arrived yet. `None` keeps the revision unset,
    // so the curve is swept again on the frame the block lands rather than being skipped for ever.
    if minted.contains(&entity) {
        return None;
    }
    mint(resources, project, entity, mesh, actions)?;
    minted.insert(entity);
    Some(false)
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
    project: &Path,
    entity: Entity,
    mesh: &kooch_blockmesh::BlockMesh,
    actions: &mut Vec<EditorAction>,
) -> Option<kooch_core::Guid> {
    let Some((path, guid)) =
        crate::actions::asset_ops::new_block_asset_in(resources, project, "Sweep", mesh)
    else {
        // Only a write that failed reaches here now that the folder is handed in.
        tracing::warn!(
            target: "kooch_editor_core::sweep",
            project = %project.display(),
            "a sweep could not write its block",
        );
        return None;
    };
    tracing::info!(
        target: "kooch_editor_core::sweep",
        file = %path.display(),
        faces = mesh.face_count(),
        "a sweep took its own block",
    );
    // 🔴 Emitted as actions, never written into this world. In a remote session the world here is
    // a mirror of the project's, and a component inserted into a mirror is gone at the next
    // snapshot — while the entity that is actually in the game never got it. An action goes down
    // whichever path the session is on, and is undoable on both.
    let mut added = 0;
    for (_, type_name) in kooch_blockmesh::block_components() {
        match component_id(resources, type_name) {
            Some(component) => {
                actions.push(EditorAction::AddComponent { entity, component });
                added += 1;
            }
            // A component the editor cannot name is a component the wire cannot carry, and an
            // action for it would be dropped without a word.
            None => tracing::warn!(
                target: "kooch_editor_core::sweep",
                type_name,
                "a block component has no id here, so the sweep cannot ask for it",
            ),
        }
    }
    tracing::info!(
        target: "kooch_editor_core::sweep",
        added,
        "asked for the components that point at the block",
    );
    let component = component_id(resources, std::any::type_name::<Block>())?;
    actions.push(EditorAction::SetField {
        entity,
        component,
        field: "source".to_owned(),
        value: kooch_ecs::reflect::ReflectValue::String(guid.to_string()),
    });
    Some(guid)
}

/// The id the actions above name a component by.
fn component_id(resources: &Resources, type_name: &str) -> Option<ComponentId> {
    resources
        .get::<kooch_ecs::component::ComponentNames>()?
        .id(type_name)
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
