//! Cylinder and cone — a side surface plus flat caps.
//!
//! The caps get their own vertices with the cap's normal, never shared
//! with the side wall. Sharing them would average a vertical normal with
//! a horizontal one and round the rim off, which on a cylinder reads as a
//! badly smoothed bevel.

use glam::{Vec2, Vec3};

use crate::mesh::Mesh;

use super::builder::{MeshBuilder, ring};

/// Cylinder along local Y, centred on the origin. Height is
/// `2 * half_height`.
pub(super) fn cylinder(radius: f32, half_height: f32, sectors: u32) -> Mesh {
    let radius = radius.max(super::MIN_EXTENT);
    let half_height = half_height.max(super::MIN_EXTENT);
    let sectors = sectors.max(super::MIN_SECTORS);

    let mut b = MeshBuilder::default();

    // Side wall: two rings with horizontal normals.
    let top = ring(radius, half_height, sectors);
    let bottom = ring(radius, -half_height, sectors);
    let side_base = b.vertices_len();
    for (s, p) in top.iter().enumerate() {
        let n = Vec3::new(p.x, 0.0, p.z);
        b.vertex(*p, n, Vec2::new(s as f32 / sectors as f32, 0.0));
    }
    for (s, p) in bottom.iter().enumerate() {
        let n = Vec3::new(p.x, 0.0, p.z);
        b.vertex(*p, n, Vec2::new(s as f32 / sectors as f32, 1.0));
    }
    super::sphere::stitch_grid(&mut b, 1, sectors, side_base);

    cap(&mut b, &top, Vec3::Y, half_height, sectors);
    cap(&mut b, &bottom, Vec3::NEG_Y, -half_height, sectors);

    b.build()
}

/// Cone along local Y with its base at `-half_height` and apex at
/// `+half_height`, centred on the origin.
///
/// The apex is one vertex per sector rather than a single shared one: a
/// cone's normal is discontinuous at the tip, so one shared apex vertex
/// would have to pick a single normal and light the whole tip flat.
pub(super) fn cone(radius: f32, half_height: f32, sectors: u32) -> Mesh {
    let radius = radius.max(super::MIN_EXTENT);
    let half_height = half_height.max(super::MIN_EXTENT);
    let sectors = sectors.max(super::MIN_SECTORS);

    let mut b = MeshBuilder::default();
    let base = ring(radius, -half_height, sectors);
    let apex = Vec3::new(0.0, half_height, 0.0);

    // Side normal: perpendicular to the slope, not to the base. The
    // slope rises `2 * half_height` over a run of `radius`, so the
    // outward normal tilts by that ratio.
    let slope = Vec2::new(2.0 * half_height, radius).normalize();
    for s in 0..sectors {
        let (p0, p1) = (base[s as usize], base[s as usize + 1]);
        let mid = ((p0 + p1) * 0.5).normalize_or_zero();
        let n0 = Vec3::new(p0.x, 0.0, p0.z).normalize_or_zero() * slope.x + Vec3::Y * slope.y;
        let n1 = Vec3::new(p1.x, 0.0, p1.z).normalize_or_zero() * slope.x + Vec3::Y * slope.y;
        let na = Vec3::new(mid.x, 0.0, mid.z).normalize_or_zero() * slope.x + Vec3::Y * slope.y;

        let u0 = s as f32 / sectors as f32;
        let u1 = (s + 1) as f32 / sectors as f32;
        let a = b.vertex(p0, n0, Vec2::new(u0, 1.0));
        let c = b.vertex(p1, n1, Vec2::new(u1, 1.0));
        let tip = b.vertex(apex, na, Vec2::new((u0 + u1) * 0.5, 0.0));
        // Base-left → apex → base-right winds counter-clockwise seen
        // from outside; going straight along the base first faces the
        // whole cone inward.
        b.triangle(a, tip, c);
    }

    cap(&mut b, &base, Vec3::NEG_Y, -half_height, sectors);
    b.build()
}

/// Triangle-fans a flat cap over `rim`, facing `normal`.
fn cap(b: &mut MeshBuilder, rim: &[Vec3], normal: Vec3, y: f32, sectors: u32) {
    let centre = b.vertex(Vec3::new(0.0, y, 0.0), normal, Vec2::splat(0.5));
    let first = b.vertices_len();
    for p in rim {
        // Planar UVs from the rim position, mapped into [0,1].
        let uv =
            Vec2::new(p.x, p.z) / (rim[0].length().max(super::MIN_EXTENT) * 2.0) + Vec2::splat(0.5);
        b.vertex(*p, normal, uv);
    }
    for s in 0..sectors {
        let (a, c) = (first + s, first + s + 1);
        // A cap facing -Y is seen from below, so its winding reverses.
        if normal.y > 0.0 {
            b.triangle(centre, c, a);
        } else {
            b.triangle(centre, a, c);
        }
    }
}

#[cfg(test)]
mod tests;
