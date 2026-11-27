//! One trimesh per planet over every held block (#1949). Rapier fixes internal edges only inside one
//! mesh, so a body rolling across separate blocks bumped on every seam. The blocks are welded and
//! the shape built on a worker; the frame swaps it in place on the same body, so a body rolling on it
//! keeps its contact instead of landing again on every rebuild.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError};
use std::time::Instant;

use glam::{IVec3, Vec3};
use kooch_ecs::hierarchy::WorldOrigin;
use kooch_physics::{BodyHandle, CollisionShape, PhysicsBackend, PreparedShape, RapierBackend};

use super::sync::{GroundColliders, GroundWire, Place};
use crate::{BLOCK_SAMPLES, BlockKey};

/// Vertices closer than this, in voxels, are one: neighbours compute their shared seam vertices in
/// their own frames, a few ulps apart.
const WELD: f32 = 1e-3;

/// Blocks from its anchor a patch may reach before it is anchored again; far from it, f32 vertices
/// lose the precision the weld needs.
const FAR: i32 = 64;

#[derive(Default)]
pub(super) struct Patch {
    pub(super) body: Option<BodyHandle>,
    /// Its blocks changed since the last merge started.
    pub(super) dirty: bool,
    building: bool,
    /// Where the body sits; kept between merges, so a rebuild changes only the shape.
    anchor: Option<Anchor>,
}

/// A merge a worker finished.
pub(super) struct Merged {
    planet: u32,
    epoch: u32,
    /// The builds it holds, so only those count as in the solver.
    members: Vec<(BlockKey, u64)>,
    shape: Option<PreparedShape>,
    place: Place,
    /// Anchored anew, so the body has to be placed again.
    moved: bool,
}

/// The block every other block's offset is measured from.
#[derive(Clone, Copy)]
struct Anchor {
    cell: IVec3,
    place: Place,
}

struct Member {
    key: BlockKey,
    id: u64,
    wire: Arc<GroundWire>,
}

/// Sends every dirty planet's blocks to a worker, one merge per planet at a time.
pub(super) fn merge(ground: &mut GroundColliders) {
    let mut jobs: HashMap<u32, (Option<Anchor>, Vec<Member>)> = HashMap::new();
    for (&planet, patch) in &ground.patches {
        if patch.dirty && !patch.building {
            jobs.insert(planet, (None, Vec::new()));
        }
    }
    if jobs.is_empty() {
        return;
    }
    for (key, held) in &ground.held {
        let (Some((anchor, members)), Some(wire)) = (jobs.get_mut(&key.planet), &held.wire) else {
            continue;
        };
        anchor.get_or_insert(Anchor {
            cell: key.cell,
            place: held.place,
        });
        members.push(Member {
            key: *key,
            id: held.id,
            wire: Arc::clone(wire),
        });
    }
    for (planet, (anchor, members)) in jobs {
        let Some(patch) = ground.patches.get_mut(&planet) else {
            continue;
        };
        patch.dirty = false;
        let Some(first) = anchor else {
            // Nothing left with a surface: the body goes without a worker.
            ground
                .merged
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(Merged {
                    planet,
                    epoch: ground.epoch,
                    members: Vec::new(),
                    shape: None,
                    place: Place::default(),
                    moved: false,
                });
            continue;
        };
        let near = |anchor: &Anchor| {
            let reach = |m: &Member| (m.key.cell - anchor.cell).abs().max_element();
            members.iter().all(|m| reach(m) <= FAR)
        };
        let kept = patch.anchor.filter(near);
        let anchor = kept.unwrap_or(first);
        patch.anchor = Some(anchor);
        patch.building = true;
        let moved = kept.is_none();
        let merged = Arc::clone(&ground.merged);
        let epoch = ground.epoch;
        rayon::spawn(move || {
            let mut done = weld(planet, epoch, anchor, members);
            done.moved = moved;
            merged
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(done);
        });
    }
}

/// On a worker: the planet's blocks as one welded trimesh, in the anchor block's frame.
fn weld(planet: u32, epoch: u32, anchor: Anchor, members: Vec<Member>) -> Merged {
    let started = Instant::now();
    let place = anchor.place;
    let step = place.transform.scale * place.side;
    let eps = (step.min_element() / BLOCK_SAMPLES as f32 * WELD).max(f32::MIN_POSITIVE);
    let mut vertices: Vec<Vec3> = Vec::new();
    let mut indices: Vec<[u32; 3]> = Vec::new();
    let mut seen: HashMap<[i64; 3], u32> = HashMap::new();
    for member in &members {
        let offset = step * (member.key.cell - anchor.cell).as_vec3();
        let remap: Vec<u32> = member
            .wire
            .vertices
            .iter()
            .map(|v| {
                let p = offset + *v;
                let q = (p / eps).round().as_i64vec3().to_array();
                *seen.entry(q).or_insert_with(|| {
                    vertices.push(p);
                    vertices.len() as u32 - 1
                })
            })
            .collect();
        for t in &member.wire.indices {
            let [a, b, c] = t.map(|i| remap.get(i as usize).copied().unwrap_or(0));
            if a != b && b != c && a != c {
                indices.push([a, b, c]);
            }
        }
    }
    let triangles = indices.len();
    let shape = (!indices.is_empty())
        .then(|| RapierBackend::prepare(&CollisionShape::TriMesh { vertices, indices }))
        .and_then(|built| {
            built
                .map_err(|error| tracing::debug!(target: "kooch_planet", "ground patch: {error}"))
                .ok()
        });
    tracing::trace!(
        target: "kooch_planet",
        planet,
        blocks = members.len(),
        triangles,
        merge_us = started.elapsed().as_micros() as u64,
        "ground patch",
    );
    Merged {
        planet,
        epoch,
        members: members.iter().map(|m| (m.key, m.id)).collect(),
        shape,
        place,
        moved: false,
    }
}

/// Swaps each finished planet's body, and marks the builds it holds as in the solver.
pub(super) fn swap(
    backend: &mut dyn PhysicsBackend,
    origin: &WorldOrigin,
    ground: &mut GroundColliders,
) {
    let merged = std::mem::take(&mut *ground.merged.lock().unwrap_or_else(PoisonError::into_inner));
    for done in merged {
        if done.epoch != ground.epoch {
            continue;
        }
        let patch = ground.patches.entry(done.planet).or_default();
        patch.building = false;
        let in_place = match (patch.body, &done.shape) {
            (Some(body), Some(shape)) => !done.moved && backend.set_prepared(body, shape),
            _ => false,
        };
        if !in_place {
            if let Some(old) = patch.body.take() {
                backend.remove_body(old);
            }
            let place = done.place;
            let position = (place.transform.position - origin.from(Some(&place.cell))).as_vec3();
            let rotation = place.transform.rotation;
            patch.body = done
                .shape
                .and_then(|shape| backend.add_prepared(&shape, position, rotation));
        }
        for (key, id) in done.members {
            if let Some(held) = ground.held.get_mut(&key).filter(|held| held.id == id) {
                held.merged = true;
            }
        }
    }
}

/// Builds a planet again whose body the solver lost.
pub(super) fn revive(backend: &dyn PhysicsBackend, ground: &mut GroundColliders) {
    for (&planet, patch) in &mut ground.patches {
        if patch.body.is_some_and(|body| !backend.contains(body)) {
            patch.body = None;
            patch.anchor = None;
            patch.dirty = true;
            let held = ground
                .held
                .iter_mut()
                .filter(|(key, _)| key.planet == planet);
            held.for_each(|(_, held)| held.merged = false);
        }
    }
}
