//! Running an extrude down a curve instead of straight out (#1526).
//!
//! Every ring is placed from the face as it was modelled, never from the ring before it: a chain
//! of relative steps drifts, and a drifting extrude is only visible once the curve is long.

use glam::{Mat3, Quat, Vec3};
use kooch_ecs::spline::{Knot, arc, eval};

use crate::BlockMesh;

use super::Extruded;

/// Ceiling on the rings one run emits, so a sample count dragged high cannot hang the editor.
const MAX_RINGS: usize = 256;

impl BlockMesh {
    /// Extrudes `faces` along the curve, one ring per sample, evenly spaced **in metres**.
    ///
    /// 🔴 The knots are read **where they are**, in this mesh's own space (#1554): the run lands on
    /// the curve the author is looking at. What the face contributes is its shape and how it sits
    /// relative to the curve's start, not where it was modelled.
    ///
    /// `taper` scales each ring by the curve's radius there, relative to the radius at the start.
    /// `None` when the selection names nothing or the curve has no length.
    pub fn extrude_along(
        &mut self,
        faces: &[u32],
        points: &[Knot],
        closed: bool,
        samples: u32,
        taper: bool,
    ) -> Option<Extruded> {
        let length = arc::length(points, closed);
        if !(length > 0.0) {
            return None;
        }
        let rings = (samples.max(1) as usize).min(MAX_RINGS);
        self.centre_of(faces)?;
        let start = eval::frame(points, closed, 0.0);
        let into_start = basis(&start).inverse();
        let at_start = start.at;
        let radius_start = eval::radius_at(points, closed, 0.0);

        // The face seen from the curve's start, read once: every ring below is placed from these,
        // so the section keeps the offset and the angle it has against the curve there.
        let plan: Vec<Vec<Vec3>> = faces
            .iter()
            .map(|face| {
                self.face(*face as usize)
                    .unwrap_or(&[])
                    .iter()
                    .map(|corner| into_start * (self.positions[*corner as usize] - at_start))
                    .collect()
            })
            .collect();

        // One table for the whole run: `arc::parameter_at` rebuilds it on every call.
        let table = arc::walked(points, closed);
        let mut walls = Vec::new();
        let mut current = faces.to_vec();
        for ring in 1..=rings {
            let distance = length * ring as f32 / rings as f32;
            let t = arc::parameter_in(&table, distance);
            let frame = eval::frame(points, closed, t);
            let rotation = basis(&frame);
            let scale = match taper && radius_start > 0.0 {
                true => eval::radius_at(points, closed, t) / radius_start,
                false => 1.0,
            };
            // Zero, then placed exactly: the walls want corners of their own, and where those land
            // is the next three lines' business, not the straight extrude's.
            let extruded = self.extrude(&current, Vec3::ZERO)?;
            walls.extend(extruded.walls);
            for (face, corners) in extruded.faces.iter().zip(&plan) {
                let moved = self.face(*face as usize)?.to_vec();
                for (corner, local) in moved.iter().zip(corners) {
                    let local = Vec3::new(local.x * scale, local.y * scale, local.z);
                    self.positions[*corner as usize] = frame.at + rotation * local;
                }
            }
            current = extruded.faces;
        }

        Some(Extruded {
            faces: current,
            walls,
        })
    }
}

/// The curve's frame as a rotation.
///
/// 🔴 Built from `up × tangent`, not from [`Frame::side`](eval::Frame::side): that one is
/// `tangent × up`, which makes `(side, up, tangent)` left-handed and mirrors whatever is carried.
fn basis(frame: &eval::Frame) -> Quat {
    let tangent = frame.tangent.normalize_or(Vec3::Z);
    let up = frame.up.normalize_or(Vec3::Y);
    let side = up.cross(tangent).normalize_or(Vec3::X);
    Quat::from_mat3(&Mat3::from_cols(side, up, tangent))
}

#[cfg(test)]
mod tests;
