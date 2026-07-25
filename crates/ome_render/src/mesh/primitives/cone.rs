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
mod tests {
    use super::*;
    use crate::mesh::primitives::tests::{
        assert_outward_facing, assert_unit_normals, assert_uvs_in_unit_range,
    };

    #[test]
    fn cylinder_fills_its_bounds_and_is_closed() {
        let mesh = cylinder(1.5, 2.0, 16);
        assert!((mesh.aabb.max.y - 2.0).abs() < 1e-4);
        assert!((mesh.aabb.min.y + 2.0).abs() < 1e-4);
        assert!((mesh.aabb.max.x - 1.5).abs() < 1e-4);
        assert_unit_normals(&mesh);
        assert_uvs_in_unit_range(&mesh);
        assert_outward_facing(&mesh);
    }

    /// Side normals are horizontal, cap normals vertical. If the rim
    /// vertices were shared they would average into neither.
    #[test]
    fn cylinder_rim_normals_are_not_averaged() {
        let mesh = cylinder(1.0, 1.0, 8);
        let horizontal = mesh
            .vertices
            .iter()
            .filter(|v| v.normal[1].abs() < 1e-4)
            .count();
        let vertical = mesh
            .vertices
            .iter()
            .filter(|v| v.normal[1].abs() > 0.99)
            .count();
        assert!(horizontal > 0, "no side normals");
        assert!(vertical > 0, "no cap normals");
        assert_eq!(
            horizontal + vertical,
            mesh.vertices.len(),
            "some rim normal was averaged between the wall and a cap"
        );
    }

    #[test]
    fn cone_has_its_apex_on_the_axis() {
        let mesh = cone(1.0, 2.0, 16);
        let apexes = mesh
            .vertices
            .iter()
            .filter(|v| (v.position[1] - 2.0).abs() < 1e-4)
            .count();
        assert_eq!(apexes, 16, "expected one apex vertex per sector");
        for v in mesh.vertices.iter().filter(|v| v.position[1] > 1.9) {
            assert!(Vec2::new(v.position[0], v.position[2]).length() < 1e-4);
        }
        assert_unit_normals(&mesh);
        assert_outward_facing(&mesh);
    }

    /// The side normal follows the slope. A cone normal that is merely
    /// horizontal lights like a cylinder.
    #[test]
    fn cone_side_normals_follow_the_slope() {
        let mesh = cone(1.0, 1.0, 16);
        let sloped = mesh
            .vertices
            .iter()
            .filter(|v| v.normal[1] > 0.1 && v.normal[1] < 0.99)
            .count();
        assert!(sloped > 0, "no normal tilts with the slope");
    }

    #[test]
    fn degenerate_dimensions_are_clamped() {
        for mesh in [cylinder(0.0, 0.0, 0), cone(0.0, 0.0, 0)] {
            assert!(mesh.index_count() > 0);
            assert_outward_facing(&mesh);
        }
    }
}
