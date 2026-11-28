//! Each planet's stamp stack (#1954): rebuilt when its list changes, and only the blocks the change
//! reaches forgotten and generated again.

use std::sync::Arc;

use super::{Found, PlanetTerrain};
use crate::Shape;
use crate::lod::{Base, bounds};
use crate::stamp::{Stack, changed, overlaps};

/// Brings every placed planet's stack up to its `Terrain::stamps`.
pub(super) fn sync_stacks(state: &mut PlanetTerrain, placed: &[Option<usize>], found: &[Found]) {
    for (index, at) in placed.iter().enumerate() {
        let Some(planet) = at.and_then(|at| found.get(at)) else {
            continue;
        };
        if state.stacks.len() <= index {
            state
                .stacks
                .resize_with(index + 1, || Arc::new(Stack::default()));
        }
        let stamps = &planet.planet.stamps;
        let base = state
            .sources
            .get(index)
            .and_then(Option::as_deref)
            .map_or(Base::Sphere, Base::of);
        let old = Arc::clone(&state.stacks[index]);
        let volumes = state.volumes.get(index).cloned().unwrap_or_default();
        if old.stamps != *stamps || old.volumes != volumes {
            let shape = Shape::new(&planet.planet, base);
            let revision = old.revision.wrapping_add(1);
            let stack = Stack::new(stamps, &volumes, base, shape.voxel, revision);
            state.stacks[index] = Arc::new(stack);
            forget(
                state,
                index as u32,
                &shape,
                changed(&old.stamps, stamps, base),
            );
        }
        if let Some(terrain) = &mut state.terrain {
            terrain.set_stamps(index as u32, &state.stacks[index].gpu);
        }
    }
}

/// Drops what the selection and the pool know of `planet` inside `boxes` (everything for `None`),
/// so the blocks there are bounded, generated and meshed again with the new stack.
fn forget(
    state: &mut PlanetTerrain,
    planet: u32,
    shape: &Shape,
    boxes: Option<Vec<(glam::DVec3, glam::DVec3)>>,
) {
    if boxes.as_ref().is_some_and(Vec::is_empty) {
        return;
    }
    state.known.retain(|key, _| {
        key.planet != planet
            || boxes.as_ref().is_some_and(|boxes| {
                let (min, max) = bounds(shape, *key);
                !boxes.iter().any(|&(lo, hi)| overlaps(min, max, lo, hi))
            })
    });
    state.rerun.force();
    if let Some(terrain) = &mut state.terrain {
        terrain.retry_within(&state.pool, planet, shape, boxes.as_deref());
    }
}
