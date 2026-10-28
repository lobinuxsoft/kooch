//! Snapping nearly-equal positions together before simplification (#1149).
//!
//! meshopt finds a UV seam by **exact** position equality. A seam written as `sin(2π)` lands
//! ~1e-7 off its partner, reads as two open borders instead of one seam, and the coarse LODs then
//! stretch triangles across it.

use std::collections::HashMap;

use crate::mesh::MeshVertex;

/// Positions closer than this are one point. Far under any visible distance, far over float noise.
const TOLERANCE: f32 = 1e-5;

/// Moves every vertex onto the first earlier one within [`TOLERANCE`]. Only positions change.
pub(super) fn weld_positions(vertices: &mut [MeshVertex]) {
    let cell = |p: [f32; 3]| p.map(|c| (c / TOLERANCE).floor() as i64);
    let mut grid: HashMap<[i64; 3], Vec<u32>> = HashMap::new();
    for index in 0..vertices.len() {
        let position = vertices[index].position;
        let [x, y, z] = cell(position);
        // The 27 neighbours, because two points a hair apart can straddle a cell edge.
        let found = (-1..=1)
            .flat_map(|dx| {
                (-1..=1).flat_map(move |dy| (-1..=1).map(move |dz| [x + dx, y + dy, z + dz]))
            })
            .filter_map(|key| grid.get(&key))
            .flatten()
            .map(|&other| vertices[other as usize].position)
            .find(|other| distance(*other, position) < TOLERANCE);
        match found {
            Some(other) => vertices[index].position = other,
            None => grid.entry([x, y, z]).or_default().push(index as u32),
        }
    }
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    glam::Vec3::from(a).distance(glam::Vec3::from(b))
}
