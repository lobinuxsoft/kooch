//! Sphere and capsule — the two shapes built from stacked rings of
//! latitude, with normals that follow the surface rather than the facets.
//!
//! Both run along local Y, which is the axis Rapier's `capsule_y` uses
//! and the axis a character controller assumes. A capsule generated
//! along Z would need a rotation baked into every collider that uses it.

use glam::{Vec2, Vec3};

use crate::mesh::Mesh;

use super::builder::{MeshBuilder, ring};

/// UV sphere centred on the origin.
///
/// `rings` counts the bands of latitude, `sectors` the columns of
/// longitude. Poles are degenerate rings rather than single vertices: a
/// shared pole vertex can carry only one UV, which pinches the texture
/// into a point and makes every column meet at the same texel.
pub(super) fn sphere(radius: f32, rings: u32, sectors: u32) -> Mesh {
    let radius = radius.max(super::MIN_EXTENT);
    let rings = rings.max(super::MIN_RINGS);
    let sectors = sectors.max(super::MIN_SECTORS);

    let mut b = MeshBuilder::with_capacity(
        ((rings + 1) * (sectors + 1)) as usize,
        (rings * sectors * 2) as usize,
    );

    for i in 0..=rings {
        let v = i as f32 / rings as f32;
        let phi = v * std::f32::consts::PI;
        let y = radius * phi.cos();
        let r = radius * phi.sin();
        for (s, p) in ring(r, y, sectors).into_iter().enumerate() {
            b.vertex(p, p, Vec2::new(s as f32 / sectors as f32, v));
        }
    }

    stitch_grid(&mut b, rings, sectors, 0);
    b.build()
}

/// Capsule along local Y: a cylinder of `2 * half_height` closed by two
/// hemispheres of `radius`. Total height is `2 * (half_height + radius)`.
pub(super) fn capsule(radius: f32, half_height: f32, rings: u32, sectors: u32) -> Mesh {
    let radius = radius.max(super::MIN_EXTENT);
    let half_height = half_height.max(0.0);
    // Rings are split between the two caps, so an odd count would make
    // one hemisphere coarser than the other.
    let cap_rings = (rings.max(super::MIN_RINGS) + 1) / 2;
    let sectors = sectors.max(super::MIN_SECTORS);

    let mut b = MeshBuilder::default();
    // Latitude rows top to bottom: the upper hemisphere, then the lower.
    // The last row of the top cap and the first of the bottom both sit at
    // the equator with the same radius, offset to ±half_height — so the
    // band between them *is* the cylindrical body, with no special case.
    let rows: Vec<(f32, f32)> = [
        (half_height, 0.0, std::f32::consts::FRAC_PI_2),
        (
            -half_height,
            std::f32::consts::FRAC_PI_2,
            std::f32::consts::PI,
        ),
    ]
    .into_iter()
    .flat_map(|(offset, from, to)| {
        (0..=cap_rings).map(move |i| {
            let t = i as f32 / cap_rings as f32;
            (offset, from + (to - from) * t)
        })
    })
    .collect();

    let last_row = (rows.len() - 1) as f32;
    for (row, &(offset, phi)) in rows.iter().enumerate() {
        let y = radius * phi.cos();
        let r = radius * phi.sin();
        // v runs 0 at the top pole to 1 at the bottom across the whole
        // capsule, so the body gets its share of the texture instead of
        // both caps claiming the full range.
        let v = row as f32 / last_row;
        // The normal belongs to the hemisphere, not to the offset
        // position: offsetting it would tilt the lighting along the body.
        for (s, p) in ring(r, y, sectors).into_iter().enumerate() {
            let u = s as f32 / sectors as f32;
            b.vertex(p + Vec3::new(0.0, offset, 0.0), p, Vec2::new(u, v));
        }
    }

    stitch_grid(&mut b, rows.len() as u32 - 1, sectors, 0);
    b.build()
}

/// Stitches a `rows + 1` by `sectors + 1` grid of already-pushed vertices
/// into triangles, starting at vertex index `base`.
///
/// Degenerate quads at the poles — where one row has zero radius — emit
/// as triangles with two coincident corners. `meshopt` drops them during
/// meshlet build, and keeping the grid uniform here is worth more than
/// special-casing two rows out of a hundred.
pub(super) fn stitch_grid(b: &mut MeshBuilder, rows: u32, sectors: u32, base: u32) {
    let stride = sectors + 1;
    for row in 0..rows {
        for s in 0..sectors {
            let top = base + row * stride + s;
            let bottom = top + stride;
            // Rows run from +Y down and sectors rotate +X towards +Z, so
            // it is (top, top+1, bottom+1, bottom) that winds
            // counter-clockwise seen from outside. Taking the columns in
            // the other order faces every quad inward — which shows up
            // only as an invisible mesh once backface culling is on,
            // never as a warning.
            b.quad(top, top + 1, bottom + 1, bottom);
        }
    }
}

#[cfg(test)]
mod tests;
