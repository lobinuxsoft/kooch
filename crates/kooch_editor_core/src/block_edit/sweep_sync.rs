//! Keeping a swept block's geometry in step with the curve it is swept along (#1152).
//!
//! 🔴 **The sweep completes its own entity.** What the author adds is a component, not a block, so
//! the first time a curve has something to sweep this asks for the block components and fills in
//! the geometry — after that it is an ordinary block, with ordinary faces, colliders and extrudes.
//!
//! Since #1452 the geometry is the component: there is no `.block` to mint, no guid to point at
//! and no file to keep in step. The scene is where a sweep's mesh lives, and saving the scene is
//! what saves it.

use std::collections::{HashMap, HashSet};

use kooch_blockmesh::{Block, SplineSweep, sweep};
use kooch_core::resource::Resources;
use kooch_ecs::component::{ComponentId, ComponentRegistry};
use kooch_ecs::entity::Entity;
use kooch_ecs::query::Query;
use kooch_ecs::reflect::Reflect;
use kooch_ecs::spline::{Spline, curve_revision};

use crate::actions::EditorAction;

/// What each sweep was last built from, as the curve's own revision mixed with its parameters.
#[derive(Default)]
pub(crate) struct BuiltSweeps {
    revisions: HashMap<Entity, u64>,
    /// Sweeps whose block components were already asked for. 🔴 Those travel as actions, so there
    /// are frames where the request is in flight and the entity has not got them yet. Asking again
    /// in one of them is a second `AddComponent` per frame for as long as the wire takes.
    asked: HashSet<Entity>,
    /// Last reported `(splines, sweeps, both)`, so the count below is said when it changes rather
    /// than every frame.
    counted: Option<(usize, usize, usize)>,
}

/// Rebuilds every sweep whose curve or parameters changed since it was last seen.
pub(crate) fn sync_sweeps(resources: &mut Resources, actions: &mut Vec<EditorAction>) {
    let mut built = resources.remove::<BuiltSweeps>().unwrap_or_default();
    count_sweeps(resources, &mut built);

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
    built.asked.retain(|entity| seen.contains(entity));

    for (entity, mesh) in changed {
        // 🔴 A rebuild that did not land is NOT a built sweep (#1481's lesson, on another system).
        // Recording the revision above is what would make a miss permanent: the curve has not
        // changed, so the guard skips it for ever and the author watches an entity with no mesh.
        if !build(resources, entity, &mesh, actions, &mut built.asked) {
            built.revisions.remove(&entity);
        }
    }
    resources.insert(built);
}

/// What this system can actually see, said when it changes.
///
/// 🔴 A sweep that builds nothing and a sweep the query never found look identical from outside,
/// and that is the difference between a bug in the mesh and a bug in the world. `splines` without
/// `sweeps` means the component did not reach this world; `both` at zero with `sweeps` above it
/// means the two are on different entities.
fn count_sweeps(resources: &Resources, built: &mut BuiltSweeps) {
    let mut counts = (0, 0, 0);
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
        "what the sweep sync can see",
    );
}

/// Puts `mesh` on the entity's block, asking for the block components first if it has none.
/// `false` when it could not land and the sweep has to be tried again next frame.
fn build(
    resources: &mut Resources,
    entity: Entity,
    mesh: &kooch_blockmesh::BlockMesh,
    actions: &mut Vec<EditorAction>,
    asked: &mut HashSet<Entity>,
) -> bool {
    if !has_block(resources, entity) {
        // Already asked, and the components have not arrived. `false` keeps the revision unset, so
        // the curve is swept again on the frame they land rather than being skipped for ever.
        if !asked.insert(entity) {
            return false;
        }
        request(resources, entity, actions);
        return false;
    }
    asked.remove(&entity);

    let Some(geometry) = Block {
        mesh: mesh.clone(),
        source: None,
    }
    .reflect_get("mesh") else {
        return false;
    };
    let Some(component) = component_id(resources, std::any::type_name::<Block>()) else {
        return false;
    };
    // 🔴 An action, never a write into this world. In a remote session the world here is a mirror
    // of the project's, and a component written into a mirror is gone at the next snapshot — while
    // the entity that is actually in the game never got it.
    actions.push(EditorAction::SetField {
        entity,
        component,
        field: "mesh".to_owned(),
        value: geometry,
    });
    tracing::info!(
        target: "kooch_editor_core::sweep",
        faces = mesh.face_count(),
        "a sweep filled in its block",
    );
    true
}

/// Whether the entity already carries the block the sweep writes to.
fn has_block(resources: &Resources, entity: Entity) -> bool {
    resources
        .get::<ComponentRegistry>()
        .and_then(|registry| registry.get_cpu::<Block>())
        .is_some_and(|storage| storage.get(entity).is_some())
}

/// Asks for every component a block needs, as actions.
fn request(resources: &Resources, entity: Entity, actions: &mut Vec<EditorAction>) {
    for (_, type_name) in kooch_blockmesh::block_components() {
        match component_id(resources, type_name) {
            Some(component) => actions.push(EditorAction::AddComponent { entity, component }),
            // A component the editor cannot name is a component the wire cannot carry, and an
            // action for it would be dropped without a word.
            None => tracing::warn!(
                target: "kooch_editor_core::sweep",
                type_name,
                "a block component has no id here, so the sweep cannot ask for it",
            ),
        }
    }
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
