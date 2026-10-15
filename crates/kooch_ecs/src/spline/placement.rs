//! Laying entities along a curve (#1428).
//!
//! Unity's `Spline Instantiate`, and the half of #1092 that is not geometry: this writes
//! transforms, never meshes, which is why it is the cheap one and the one a level needs first.
//!
//! 🔴 **The instances are regenerated, not saved.** They carry no [`SceneMember`], which is what
//! the scene writer walks, so none of them reaches the file and the curve is the only description
//! of where they are. The cost the author pays is that no single instance can be hand-edited —
//! move the curve or change the numbers instead.
//!
//! 🔴 **A run is rebuilt only when its description changes.** Not once a frame: a collectible the
//! player picks up despawns itself, and a placement that rebuilt continuously would put it
//! straight back. The revision below is what tells "the author moved a knot" apart from "the game
//! took one away".

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use glam::{Quat, Vec3};

use kooch_core::Guid;
use kooch_core::resource::Resources;

use crate::component::{Component, ComponentRegistry};
use crate::entity::Entity;
use crate::query::Query;
use crate::transform::Transform;

use super::Knot;

/// Spread `count` evenly along the whole curve.
pub const SPACING_COUNT: u32 = 0;
/// One every `spacing` units of arc length.
pub const SPACING_DISTANCE: u32 = 1;
/// One at each control point, ignoring both numbers.
pub const SPACING_KNOTS: u32 = 2;

/// What the Inspector offers for `mode`.
pub static SPACING_CHOICES: &[crate::reflect::FieldChoice] = &[
    crate::reflect::FieldChoice {
        label: "Count",
        value: SPACING_COUNT as i64,
    },
    crate::reflect::FieldChoice {
        label: "Distance",
        value: SPACING_DISTANCE as i64,
    },
    crate::reflect::FieldChoice {
        label: "At knots",
        value: SPACING_KNOTS as i64,
    },
];

/// Lower bound on `spacing`, so a zero never divides a curve into infinite instances.
const MIN_SPACING: f32 = 0.01;

/// Ceiling on one run, so a spacing typed as `0.001` cannot hang the editor before the author
/// finishes typing the rest of the number.
const MAX_INSTANCES: usize = 4096;

/// What to lay along the [`Spline`](super::Spline) on the same entity, and how often.
#[derive(Debug, Clone, Default, crate::Reflect)]
pub struct SplinePlacement {
    /// The prefab stamped at each step. Nothing is placed until this names one.
    #[reflect(asset = "kooch_ecs::scene::document::SceneDocument")]
    pub prefab: Option<Guid>,
    #[reflect(choices = SPACING_CHOICES)]
    pub mode: u32,
    /// How many, when `mode` is `Count`.
    pub count: u32,
    /// Units of arc length between instances, when `mode` is `Distance`.
    pub spacing: f32,
    /// Moved from the curve, in the instance's own axes — so a lamp sits beside the path rather
    /// than on it, and keeps doing so around a corner.
    pub offset: Vec3,
}

impl Component for SplinePlacement {}

/// An entity laid down by a placement, so the next rebuild knows what to take away.
///
/// ⚠️ Holds an `Entity`, which a scene file cannot carry across a reload (#712). Harmless here
/// precisely because this is never written to one.
#[derive(Debug, Clone, Default, crate::Reflect)]
pub struct Placed {
    pub source: Entity,
}

impl Component for Placed {}

/// One run, and the description it was built from.
struct Run {
    revision: u64,
    /// What each instance was stamped from, so a rebuild can tell "the same run, moved" from "a
    /// different run".
    prefab: Guid,
    placed: Vec<Entity>,
}

/// What each placement has already laid down.
///
/// A `HashMap` keyed by the source entity: this is CPU-only bookkeeping that runs when an author
/// edits, not a per-frame walk, so the rule against maps in hot paths does not reach it.
#[derive(Default)]
pub struct PlacedRuns {
    runs: HashMap<Entity, Run>,
    /// Sources already reported as unable to lay their prefab, so the per-frame retry does not
    /// become a per-frame log.
    failed: std::collections::HashSet<Entity>,
}

/// Rebuilds every placement whose description changed.
pub fn place_along_splines(resources: &mut Resources) {
    let wanted = gather(resources);

    // 🔴 A run whose placement is gone has to go with it. The spline may have been deleted, the
    // component removed, or the whole scene replaced — in every case the instances are still
    // standing there with nothing describing them, and nothing else will ever come back for them.
    let orphans: Vec<Entity> = match resources.get::<PlacedRuns>() {
        Some(runs) => runs
            .runs
            .keys()
            .copied()
            .filter(|source| !wanted.iter().any(|(entity, ..)| entity == source))
            .collect(),
        None => Vec::new(),
    };
    for source in orphans {
        clear_run(resources, source);
    }

    // What actually changed, decided before any curve is laid out. 🔴 One pass for all of them
    // afterwards: asking per placement walked the whole query each time, which is quadratic in the
    // number of curves in the scene — and a scene load changes every one of them at once, which is
    // exactly when the quadratic bites.
    let changing: Vec<(Entity, u64, Guid, Option<(u64, Guid, usize)>)> = wanted
        .into_iter()
        .filter_map(|(source, revision, prefab)| {
            let standing = resources
                .get::<PlacedRuns>()
                .and_then(|runs| runs.runs.get(&source))
                .map(|run| (run.revision, run.prefab, run.placed.len()));
            match standing.is_some_and(|(built, ..)| built == revision) {
                true => None,
                false => Some((source, revision, prefab, standing)),
            }
        })
        .collect();
    if changing.is_empty() {
        return;
    }
    let mut laid_out = poses_for(
        resources,
        &changing.iter().map(|(e, ..)| *e).collect::<Vec<_>>(),
    );

    for (source, revision, prefab, standing) in changing {
        let poses = laid_out.remove(&source).unwrap_or_default();

        // 🔴 The same run, moved. Despawning and respawning on every edit burns an entity
        // generation per instance per keystroke, and dragging a knot is a hundred edits (#1467,
        // and #1300 is the same climb from the other direction).
        let wanted_count = poses.len();
        let placed = match standing {
            Some((_, was, many)) if was == prefab && many == wanted_count => {
                move_run(resources, source, &poses)
            }
            _ => {
                clear_run(resources, source);
                let mut placed = Vec::with_capacity(wanted_count);
                for pose in poses {
                    match stamp(resources, prefab, source, pose) {
                        Ok(entity) => placed.push(entity),
                        Err(detail) => {
                            report_failure(resources, source, prefab, &detail);
                            break;
                        }
                    }
                }
                placed
            }
        };

        // 🔴 A run that could not place everything is NOT a built run (#1481). Recording the
        // revision here is what made the failure permanent: the curve has not changed, so the
        // guard above skips it forever and the author has no reason to suspect anything.
        //
        // A prefab resolves on demand, and the first `Update` after a scene load is exactly when
        // it may not have arrived yet. Leaving the run unrecorded costs one retry per frame until
        // it does, and buys a placement that works on the frame its asset lands.
        if placed.len() != wanted_count {
            clear_run(resources, source);
            continue;
        }
        forget_failure(resources, source);

        let run = Run {
            revision,
            prefab,
            placed,
        };
        if let Some(runs) = resources.get_mut::<PlacedRuns>() {
            runs.runs.insert(source, run);
        } else {
            let mut runs = PlacedRuns::default();
            runs.runs.insert(source, run);
            resources.insert(runs);
        }
    }
}

/// Rewrites a standing run's transforms, keeping every entity it already has.
fn move_run(resources: &mut Resources, source: Entity, poses: &[Transform]) -> Vec<Entity> {
    let standing = match resources
        .get::<PlacedRuns>()
        .and_then(|r| r.runs.get(&source))
    {
        Some(run) => run.placed.clone(),
        None => return Vec::new(),
    };
    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<Transform>()
    {
        for (entity, pose) in standing.iter().zip(poses) {
            if let Some(held) = storage.get_mut(*entity) {
                *held = *pose;
            }
        }
    }
    standing
}

/// Every placement that has a curve to lay along, and what it is currently asking for.
///
/// ⚠️ Deliberately does NOT compute the poses. This runs every frame and almost every frame the
/// answer is "nothing changed" — laying out a curve to find that out is work thrown away.
fn gather(resources: &Resources) -> Vec<(Entity, u64, Guid)> {
    let query = Query::<(&super::Spline, &SplinePlacement)>::new(resources);
    let mut out = Vec::new();
    query.for_each_entity(|entity, (spline, placement)| {
        let Some(prefab) = placement.prefab else {
            return;
        };
        out.push((entity, revision(spline, placement), prefab));
    });
    out
}

/// The poses every placement in `sources` asks for, in one walk.
///
/// 🔴 One walk for all of them, not one walk each. Laying out a curve is the expensive half of
/// this module — `eval::up` transports a frame from the start of the curve for every instance —
/// so it happens only for the sources whose revision actually moved.
fn poses_for(resources: &Resources, sources: &[Entity]) -> HashMap<Entity, Vec<Transform>> {
    let query = Query::<(&super::Spline, &SplinePlacement)>::new(resources);
    let mut out = HashMap::with_capacity(sources.len());
    query.for_each_entity(|entity, (spline, placement)| {
        if sources.contains(&entity) {
            out.insert(entity, poses(spline, placement));
        }
    });
    out
}

/// Where each instance goes, in the spline entity's own space.
fn poses(spline: &super::Spline, placement: &SplinePlacement) -> Vec<Transform> {
    let points = &spline.points;
    if points.len() < 2 {
        return Vec::new();
    }
    let total = super::arc::length(points, spline.closed);

    let distances: Vec<f32> = match placement.mode {
        SPACING_KNOTS => knot_distances(points, spline.closed),
        SPACING_DISTANCE => {
            let step = placement.spacing.max(MIN_SPACING);
            // `!(total > 0.0)` rather than `<= 0.0`: a NaN length must take this branch, and every
            // comparison against NaN is false.
            if !(total > 0.0) {
                return Vec::new();
            }
            let many = ((total / step).floor() as usize + 1).min(MAX_INSTANCES);
            (0..many).map(|i| i as f32 * step).collect()
        }
        _ => {
            let many = (placement.count as usize).min(MAX_INSTANCES);
            match many {
                0 => Vec::new(),
                1 => vec![0.0],
                // A closed curve's last instance would land on its first, so the step divides by
                // the count rather than by the gaps between them.
                _ => {
                    let gaps = match spline.closed {
                        true => many,
                        false => many - 1,
                    };
                    (0..many).map(|i| total * i as f32 / gaps as f32).collect()
                }
            }
        }
    };

    distances
        .into_iter()
        .map(|distance| pose_at(spline, placement, distance))
        .collect()
}

/// The arc length each control point sits at.
fn knot_distances(points: &[Knot], closed: bool) -> Vec<f32> {
    let walked = super::arc::walked(points, closed);
    if walked.is_empty() {
        return Vec::new();
    }
    let per_segment = super::arc::STEPS_PER_SEGMENT as usize;
    walked
        .iter()
        .copied()
        .step_by(per_segment)
        .take(MAX_INSTANCES)
        .collect()
}

/// One instance's local pose.
fn pose_at(spline: &super::Spline, placement: &SplinePlacement, distance: f32) -> Transform {
    let points = &spline.points;
    let t = super::arc::parameter_at(points, spline.closed, distance);
    let position = super::eval::at(points, spline.closed, t);

    // 🔴 Always turned to the curve (#1472). The knots' `roll` lives only in this frame's up, so a
    // placement that skipped it threw the roll away and said nothing — the feature's default path
    // was the one that did not work.
    let forward = super::eval::tangent(points, spline.closed, t).normalize_or(Vec3::Z);
    let up = super::eval::up(points, spline.closed, t).normalize_or(Vec3::Y);
    let rotation = look_along(forward, up);

    Transform {
        // In the instance's own axes, so an offset holds its side of the path around a corner
        // instead of swinging across it.
        position: position + rotation * placement.offset,
        rotation,
        scale: Vec3::ONE,
    }
}

/// A rotation whose -Z looks along `forward`, the engine's convention for facing.
fn look_along(forward: Vec3, up: Vec3) -> Quat {
    let right = forward.cross(up);
    // Parallel forward and up leave no right to build a basis from; the curve's own up is the one
    // thing guaranteed not to be parallel to its tangent, so this only fires on a degenerate frame.
    if !(right.length_squared() > 1e-12) {
        return Quat::IDENTITY;
    }
    let right = right.normalize();
    let up = right.cross(forward).normalize();
    Quat::from_mat3(&glam::Mat3::from_cols(right, up, -forward))
}

/// What the run was built from, as one number.
///
/// 🔴 This is the whole mechanism that keeps a pickup picked up. It covers the curve and the
/// numbers and nothing else: despawning an instance does not change it, so the run is not rebuilt.
fn revision(spline: &super::Spline, placement: &SplinePlacement) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    spline.closed.hash(&mut hasher);
    for knot in &spline.points {
        // Every field that can bend the curve, including the authored tangents. They are ignored
        // while a knot is `Auto` — `eval` derives both — but a run must follow an `Aligned` or
        // `Broken` knot whose grips were dragged.
        for value in [
            knot.position.x,
            knot.position.y,
            knot.position.z,
            knot.roll,
            knot.arriving.x,
            knot.arriving.y,
            knot.arriving.z,
            knot.leaving.x,
            knot.leaving.y,
            knot.leaving.z,
        ] {
            value.to_bits().hash(&mut hasher);
        }
        knot.mode.hash(&mut hasher);
    }
    placement.prefab.hash(&mut hasher);
    placement.mode.hash(&mut hasher);
    placement.count.hash(&mut hasher);
    placement.spacing.to_bits().hash(&mut hasher);
    for value in [placement.offset.x, placement.offset.y, placement.offset.z] {
        value.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

/// Takes away what a placement laid down last time.
fn clear_run(resources: &mut Resources, source: Entity) {
    let Some(run) = resources.get_mut::<PlacedRuns>().and_then(|runs| {
        // Dropped together: a source whose run is gone is a source nothing will report about, and
        // a set that only ever grows is a leak however small each entry is.
        runs.failed.remove(&source);
        runs.runs.remove(&source)
    }) else {
        return;
    };
    // The whole subtree, through the one despawn that knows every store an entity is in — a
    // second removal path is a second chance to leave a dangling row behind.
    let doomed: Vec<Entity> = match resources.get::<ComponentRegistry>() {
        Some(registry) => run
            .placed
            .iter()
            .flat_map(|entity| crate::hierarchy::collect_descendants(*entity, registry))
            .collect(),
        None => run.placed,
    };
    crate::scene::sync::despawn_entities(resources, &doomed);
}

/// Stamps one instance and marks it as this placement's. `Err` carries why, for the caller to
/// report once rather than per frame.
///
/// 🔴 Never `.ok()?`. Swallowing `PrefabUnavailable` here is what let a placement lay nothing and
/// say nothing (#1481) — the one failure an author cannot see from the outside.
fn stamp(
    resources: &mut Resources,
    prefab: Guid,
    source: Entity,
    pose: Transform,
) -> Result<Entity, String> {
    let (root, members) =
        crate::scene::spawn_prefab_members(prefab, resources).map_err(|error| error.to_string())?;
    // 🔴 Spawning builds the entities; attaching is what makes them an INSTANCE of the prefab.
    // Without it a run of prefabs reads as a run of loose copies, with no link back to what they
    // came from (#1467).
    crate::prefab_instance::attach(resources, root, &members, prefab);
    // Parented to the curve, so the whole run moves with the entity that describes it and the
    // pose above stays in the space it was computed in.
    crate::hierarchy::reparent(resources, root, Some(source));

    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<Transform>()
        && let Some(held) = storage.get_mut(root)
    {
        *held = pose;
    }
    // 🔴 Every member, not just the root. The marker is what makes an archetype ephemeral, and a
    // prefab of several entities would otherwise have its children adopted and written to the
    // scene while the root stayed out — half a run in the file is worse than all of it.
    mark(resources, root, source);
    for member in members {
        mark(resources, member, source);
    }
    Ok(root)
}

/// Says why a run could not be laid — once per source, not once per frame.
///
/// The retry above runs every frame until the prefab arrives, so an unguarded log would be a
/// console full of the same line, which is the same as no log at all.
fn report_failure(resources: &mut Resources, source: Entity, prefab: Guid, detail: &str) {
    let Some(runs) = resources.get_mut::<PlacedRuns>() else {
        return;
    };
    if !runs.failed.insert(source) {
        return;
    }
    tracing::warn!(
        target: "kooch_ecs::spline",
        %prefab,
        detail,
        "a placement could not lay its prefab; retrying every frame until it loads",
    );
}

/// Clears the reported mark once a run succeeds, so a later failure is heard again.
fn forget_failure(resources: &mut Resources, source: Entity) {
    if let Some(runs) = resources.get_mut::<PlacedRuns>()
        && runs.failed.remove(&source)
    {
        tracing::info!(
            target: "kooch_ecs::spline",
            "a placement that could not find its prefab has laid its run",
        );
    }
}

/// Adds the [`Placed`] marker, registering its storage on first use.
///
/// 🔴 The ARCHETYPE too, not just the storage. Whether something is ephemeral is decided per
/// archetype — `ephemeral.intersects(archetype.components())` — so a marker written only into its
/// own column is a marker nothing that matters ever sees.
fn mark(resources: &mut Resources, entity: Entity, source: Entity) {
    if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
        registry.register_cpu_reflected::<Placed>();
        if let Some(storage) = registry.get_cpu_mut::<Placed>() {
            storage.insert(entity, Placed { source });
        }
    }
    if let Some(archetypes) = resources.get_mut::<crate::archetype_registry::ArchetypeRegistry>()
        && let Some(current) = archetypes.entity_archetype(entity)
    {
        let next =
            archetypes.archetype_after_add_dynamic(current, std::any::TypeId::of::<Placed>());
        archetypes.register_entity(entity, next);
    }
}

#[cfg(test)]
mod tests;
