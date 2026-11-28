//! A terrain's stamps reaching the generate pass, and the blocks an edit of them makes stale.

use std::sync::Arc;

use glam::DVec3;

use super::Pipeline;
use crate::lod::{bounds, voxel_size};
use crate::mesh::Part;
use crate::stamp::{GpuStamp, overlaps};
use crate::{BlockPool, Shape};

impl Pipeline {
    /// Generates `planet` with `stamps` from now on. Nothing already generated changes until
    /// [`retry_within`](Self::retry_within) queues it.
    pub(crate) fn set_stamps(&mut self, planet: u32, stamps: &Arc<[GpuStamp]>) {
        self.stamps.set(planet, stamps);
    }

    /// Queues again every resident block of `planet` within two of its voxels of a box, as one
    /// change: the old mesh stays drawn until the new one is ready. `None` queues them all.
    pub(crate) fn retry_within(
        &mut self,
        pool: &BlockPool,
        planet: u32,
        shape: &Shape,
        boxes: Option<&[(DVec3, DVec3)]>,
    ) {
        self.epoch += 1;
        let epoch = self.epoch;
        let slots: Vec<u32> = pool
            .slots_of(planet)
            .filter(|&slot| {
                let Some(boxes) = boxes else {
                    return true;
                };
                pool.key(slot).is_some_and(|key| {
                    let (min, max) = bounds(shape, key);
                    let pad = DVec3::splat(2.0 * voxel_size(shape, key.level));
                    boxes
                        .iter()
                        .any(|&(lo, hi)| overlaps(min - pad, max + pad, lo, hi))
                })
            })
            .collect();
        for slot in slots {
            self.remesh(slot, Part::Interior, epoch);
        }
    }
}
