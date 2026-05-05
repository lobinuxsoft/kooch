//! Test helpers shared across `single_lod`, `lod_chain`, and
//! `grouping` test modules. `#[cfg(test)]` only — no runtime cost.

use crate::mesh::{Mesh, MeshVertex};

/// Single-vertex factory with a default Y-up normal. Keeps the
/// noisy `MeshVertex { position, normal, uv }` boilerplate out of
/// every test case.
pub(crate) fn vertex(p: [f32; 3]) -> MeshVertex {
    MeshVertex {
        position: p,
        normal: [0.0, 1.0, 0.0],
        uv: [0.0, 0.0],
    }
}

/// Triangulated grid (`subdivisions × subdivisions` quads) on the
/// XY plane spanning `[0, 1]²`. Used throughout the chain tests
/// because `meshopt::simplify` needs a dense surface to actually
/// reduce.
pub(crate) fn make_grid_mesh(subdivisions: usize) -> Mesh {
    let n = subdivisions + 1;
    let mut verts = Vec::with_capacity(n * n);
    for y in 0..n {
        for x in 0..n {
            verts.push(vertex([
                x as f32 / subdivisions as f32,
                y as f32 / subdivisions as f32,
                0.0,
            ]));
        }
    }
    let mut idx = Vec::with_capacity(subdivisions * subdivisions * 6);
    for y in 0..subdivisions {
        for x in 0..subdivisions {
            let a = (y * n + x) as u32;
            let b = a + 1;
            let c = a + n as u32;
            let d = c + 1;
            idx.extend_from_slice(&[a, b, c, b, d, c]);
        }
    }
    Mesh::from_arrays(verts, idx)
}
