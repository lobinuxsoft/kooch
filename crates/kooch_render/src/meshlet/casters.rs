//! Every row as the shadow caches see it, worked from the rows that changed (#2014): a world-space
//! sphere, and a stamp that changes whenever the row does, in place of a hash of its bytes.

use glam::Vec3;

use super::scene::{INSTANCE_EMPTY, INSTANCE_MASKED, INSTANCE_TRANSPARENT, MeshInstance};
use crate::shadow::InstanceBounds;

const SEE_THROUGH: u32 = INSTANCE_TRANSPARENT | INSTANCE_MASKED;

#[derive(Default)]
pub(crate) struct Casters {
    bounds: Vec<InstanceBounds>,
    /// Rows that may cover with time, checked every frame.
    see_through: Vec<u32>,
    stamp: u64,
    moved: Vec<[f32; 4]>,
}

impl Casters {
    /// Updates the rows in `changed` (ascending), drops those past the end, and re-stamps every
    /// see-through row `animated` says covers with time: its pages and cube faces redraw, or its
    /// shadow freezes at the frame they were cached (#1224).
    pub fn update(
        &mut self,
        rows: &[MeshInstance],
        changed: &[u32],
        sphere: impl Fn(&MeshInstance) -> (Vec3, f32),
        animated: impl Fn(&MeshInstance) -> bool,
    ) {
        self.moved.clear();
        let kept = rows.len().min(self.bounds.len());
        for gone in self.bounds.drain(kept..) {
            push(&mut self.moved, gone);
        }
        self.see_through.retain(|&i| (i as usize) < rows.len());
        for &i in changed {
            let Some(row) = rows.get(i as usize) else {
                continue;
            };
            let (center, radius) = match row.flags & INSTANCE_EMPTY {
                0 => sphere(row),
                _ => (Vec3::ZERO, 0.0),
            };
            self.stamp += 1;
            let now = InstanceBounds {
                center,
                radius,
                hash: self.stamp,
            };
            // Ascending, and every row past the old end changed, so a push lands at its index.
            match self.bounds.get_mut(i as usize) {
                Some(was) => push(&mut self.moved, std::mem::replace(was, now)),
                None => self.bounds.push(now),
            }
            push(&mut self.moved, now);
            let listed = self.see_through.binary_search(&i);
            match (row.flags & SEE_THROUGH != 0, listed) {
                (true, Err(at)) => self.see_through.insert(at, i),
                (false, Ok(at)) => {
                    self.see_through.remove(at);
                }
                _ => {}
            }
        }
        for &i in &self.see_through {
            let (Some(row), Some(bounds)) = (rows.get(i as usize), self.bounds.get_mut(i as usize))
            else {
                continue;
            };
            if animated(row) {
                self.stamp += 1;
                bounds.hash = self.stamp;
                push(&mut self.moved, *bounds);
            }
        }
    }

    /// One per row, in row order.
    pub fn bounds(&self) -> &[InstanceBounds] {
        &self.bounds
    }

    /// This frame's moved-caster spheres, old and new bounds alike, for the page cache (#477).
    pub fn moved(&self) -> &[[f32; 4]] {
        &self.moved
    }
}

/// An empty row reaches nothing, so it moves no page.
fn push(moved: &mut Vec<[f32; 4]>, bounds: InstanceBounds) {
    if bounds.radius > 0.0 {
        let c = bounds.center;
        moved.push([c.x, c.y, c.z, bounds.radius]);
    }
}

#[cfg(test)]
mod tests;
