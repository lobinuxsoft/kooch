//! Every row as the shadow caches see it, worked from the rows that changed (#2014): a world-space
//! sphere, and a stamp that changes whenever the row does, in place of a hash of its bytes.

use glam::{DVec3, Vec3};

use super::scene::{INSTANCE_EMPTY, INSTANCE_MASKED, INSTANCE_TRANSPARENT, MeshInstance};
use crate::shadow::InstanceBounds;

const SEE_THROUGH: u32 = INSTANCE_TRANSPARENT | INSTANCE_MASKED;

#[derive(Default)]
pub(crate) struct Casters {
    bounds: Vec<InstanceBounds>,
    /// Each row's mesh and flags, beside `bounds`: a re-set row with them unchanged only moved.
    shape: Vec<[u32; 2]>,
    /// Rows that may cover with time, checked every frame.
    see_through: Vec<u32>,
    stamp: u64,
    moved: Vec<[f32; 4]>,
    /// The floating origin the bounds are relative to.
    origin: DVec3,
    tally: Tally,
}

/// One second of page invalidation, said once a second (#2009): whole-table voids against moved
/// spheres, which redraw only the pages they reach.
#[derive(Default)]
struct Tally {
    since: Option<std::time::Instant>,
    frames: u32,
    voids: u32,
    spheres: u64,
    widest: f32,
}

impl Casters {
    /// Updates the rows in `changed` (ascending), drops those past the end, and re-stamps every
    /// see-through row `animated` says covers with time: its pages and cube faces redraw, or its
    /// shadow freezes at the frame they were cached (#1224). A rebase to `origin` moves no caster:
    /// the kept bounds follow it, and a row whose only change is the shift reaches no page (#2009).
    pub fn update(
        &mut self,
        rows: &[MeshInstance],
        changed: &[u32],
        origin: DVec3,
        sphere: impl Fn(&MeshInstance) -> (Vec3, f32),
        animated: impl Fn(&MeshInstance) -> bool,
    ) {
        self.moved.clear();
        let shift = (self.origin - origin).as_vec3();
        self.origin = origin;
        if shift != Vec3::ZERO {
            for bounds in &mut self.bounds {
                bounds.center += shift;
            }
        }
        let kept = rows.len().min(self.bounds.len());
        self.shape.truncate(kept);
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
            let shape = [row.mesh_id, row.flags];
            let kept = self.shape.get(i as usize) == Some(&shape);
            // Ascending, and every row past the old end changed, so a push lands at its index.
            match self.bounds.get_mut(i as usize) {
                Some(was) if kept && shift != Vec3::ZERO && same(was, center, radius) => {
                    was.center = center;
                }
                Some(was) => {
                    if let Some(at) = self.shape.get_mut(i as usize) {
                        *at = shape;
                    }
                    self.stamp += 1;
                    let now = InstanceBounds {
                        center,
                        radius,
                        hash: self.stamp,
                    };
                    push(&mut self.moved, std::mem::replace(was, now));
                    push(&mut self.moved, now);
                }
                None => {
                    self.shape.push(shape);
                    self.stamp += 1;
                    let now = InstanceBounds {
                        center,
                        radius,
                        hash: self.stamp,
                    };
                    self.bounds.push(now);
                    push(&mut self.moved, now);
                }
            }
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

    /// Counts this frame toward the once-a-second report; `voided` when the whole table went.
    pub fn report(&mut self, voided: bool) {
        let tally = &mut self.tally;
        let now = std::time::Instant::now();
        let since = *tally.since.get_or_insert(now);
        tally.frames += 1;
        tally.voids += u32::from(voided);
        tally.spheres += self.moved.len() as u64;
        for sphere in &self.moved {
            tally.widest = tally.widest.max(sphere[3]);
        }
        if now.duration_since(since).as_secs_f32() < 1.0 {
            return;
        }
        tracing::trace!(
            target: "kooch_render::shadow",
            frames = tally.frames,
            voids = tally.voids,
            spheres_per_frame = tally.spheres / u64::from(tally.frames.max(1)),
            widest_m = tally.widest,
            "page invalidation, last second",
        );
        *tally = Tally {
            since: Some(now),
            ..Tally::default()
        };
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

/// Whether a re-set row landed where its kept bounds already are: the f32 rounding of a rebase,
/// a few ulps of the centre, is not a move.
fn same(was: &InstanceBounds, center: Vec3, radius: f32) -> bool {
    let slack = 1e-3 + center.abs().max_element() * 4e-7;
    was.center.distance(center) <= slack && (was.radius - radius).abs() <= slack
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
