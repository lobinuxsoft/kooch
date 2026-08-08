//! Cube and quad — the two primitives with no curvature.
//!
//! Both duplicate vertices per face rather than sharing corners. A cube
//! with eight shared vertices has to average three perpendicular normals
//! at every corner, which rounds the lighting off the edges and makes a
//! box look like a beanbag. Twenty-four vertices buy flat faces.

use glam::{Vec2, Vec3};

use crate::mesh::Mesh;

use super::builder::MeshBuilder;

/// Axis-aligned box centred on the origin.
pub(super) fn cube(half_extents: Vec3) -> Mesh {
    let h = half_extents.max(Vec3::splat(super::MIN_EXTENT));
    let mut b = MeshBuilder::with_capacity(24, 12);

    // (normal, tangent, bitangent) per face. The quad is built as
    // -t-bt, +t-bt, +t+bt, -t+bt, which winds counter-clockwise when
    // viewed from along +normal for a right-handed (t, bt, n) basis.
    let faces = [
        (Vec3::X, Vec3::NEG_Z, Vec3::Y),
        (Vec3::NEG_X, Vec3::Z, Vec3::Y),
        (Vec3::Y, Vec3::X, Vec3::NEG_Z),
        (Vec3::NEG_Y, Vec3::X, Vec3::Z),
        (Vec3::Z, Vec3::X, Vec3::Y),
        (Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y),
    ];

    for (normal, tangent, bitangent) in faces {
        let centre = normal * h;
        let t = tangent * h;
        let bt = bitangent * h;
        let a = b.vertex(centre - t - bt, normal, Vec2::new(0.0, 1.0));
        let c = b.vertex(centre + t - bt, normal, Vec2::new(1.0, 1.0));
        let d = b.vertex(centre + t + bt, normal, Vec2::new(1.0, 0.0));
        let e = b.vertex(centre - t + bt, normal, Vec2::new(0.0, 0.0));
        b.quad(a, c, d, e);
    }

    b.build()
}

/// Single-sided plane on XZ, facing +Y, centred on the origin.
///
/// Y up and facing up because its job is being a floor.
pub(super) fn quad(half_extents: Vec2) -> Mesh {
    let h = half_extents.max(Vec2::splat(super::MIN_EXTENT));
    let mut b = MeshBuilder::with_capacity(4, 2);

    // Seen from +Y looking down, -Z is "up" on screen, so winding
    // -x-z → +x-z → +x+z → -x+z is counter-clockwise from above.
    let a = b.vertex(Vec3::new(-h.x, 0.0, -h.y), Vec3::Y, Vec2::new(0.0, 0.0));
    let c = b.vertex(Vec3::new(h.x, 0.0, -h.y), Vec3::Y, Vec2::new(1.0, 0.0));
    let d = b.vertex(Vec3::new(h.x, 0.0, h.y), Vec3::Y, Vec2::new(1.0, 1.0));
    let e = b.vertex(Vec3::new(-h.x, 0.0, h.y), Vec3::Y, Vec2::new(0.0, 1.0));
    b.quad(a, e, d, c);

    b.build()
}

#[cfg(test)]
mod tests;
