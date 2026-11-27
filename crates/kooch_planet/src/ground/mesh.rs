//! Dual contouring of one block on the CPU, as `nets.wgsl` does it for a block whose neighbours are
//! all of its level: the same corners, the same QEF in f32, the same quads. Collision near a body is
//! level 0 all around it, so the seams to coarser leaves never arise.

use glam::{Mat3, Vec3};

use crate::TILE_SAMPLES;

const TILE: usize = TILE_SAMPLES as usize;
/// Cells between the tile's samples; a cell's corners are samples `c..=c + 1`.
const CELLS: usize = TILE - 1;
/// Mirrors `QEF_BIAS` in `nets.wgsl`.
const QEF_BIAS: f32 = 0.05;
const NONE: u32 = u32::MAX;

/// A block's ground as triangles, in metres from the block's corner (its sample 1).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GroundMesh {
    pub vertices: Vec<Vec3>,
    pub indices: Vec<[u32; 3]>,
}

/// Meshes a tile of `TILE_SAMPLES³` samples, x fastest, in voxels as the atlas stores them.
pub fn mesh_block(samples: &[f32], voxel: f32) -> GroundMesh {
    let mut mesh = GroundMesh::default();
    if samples.len() != TILE * TILE * TILE {
        return mesh;
    }
    let at = |s: [usize; 3]| samples[s[0] + TILE * (s[1] + TILE * s[2])];
    let mut vertex = vec![NONE; CELLS * CELLS * CELLS];
    // Every sign-changing edge whose start is one of the block's samples (1..=16) gives a quad over
    // the four cells around it, so a neighbour of the same level emits the rest.
    for z in 1..=16 {
        for y in 1..=16 {
            for x in 1..=16 {
                let s = [x, y, z];
                for axis in 0..3 {
                    let mut e = s;
                    e[axis] += 1;
                    let (a, b) = (at(s), at(e));
                    if (a < 0.0) == (b < 0.0) {
                        continue;
                    }
                    let q =
                        quad(s, axis).map(|c| cell_vertex(c, &at, &mut vertex, &mut mesh, voxel));
                    // Starting inside, the surface faces +axis; outside, the other way.
                    mesh.indices.extend(match a < 0.0 {
                        true => [[q[0], q[1], q[2]], [q[0], q[2], q[3]]],
                        false => [[q[0], q[2], q[1]], [q[0], q[3], q[2]]],
                    });
                }
            }
        }
    }
    mesh
}

/// The four cells around the edge from `s` along `axis`; u × v = axis.
fn quad(s: [usize; 3], axis: usize) -> [[usize; 3]; 4] {
    let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
    let step = |c: [usize; 3], du: usize, dv: usize| {
        let mut c = c;
        c[u] -= du;
        c[v] -= dv;
        c
    };
    [step(s, 1, 1), step(s, 0, 1), s, step(s, 1, 0)]
}

fn cell_vertex(
    c: [usize; 3],
    at: &impl Fn([usize; 3]) -> f32,
    vertex: &mut [u32],
    mesh: &mut GroundMesh,
    voxel: f32,
) -> u32 {
    let i = c[0] + CELLS * (c[1] + CELLS * c[2]);
    if vertex[i] == NONE {
        let v: [f32; 8] =
            std::array::from_fn(|k| at([c[0] + (k & 1), c[1] + ((k >> 1) & 1), c[2] + (k >> 2)]));
        let cell = Vec3::new(c[0] as f32, c[1] as f32, c[2] as f32);
        vertex[i] = mesh.vertices.len() as u32;
        // Sample 1 is the block's corner.
        mesh.vertices.push((cell + place(v) - Vec3::ONE) * voxel);
    }
    vertex[i]
}

/// `trilinear_gradient` in `nets.wgsl`.
fn gradient(v: [f32; 8], p: Vec3) -> Vec3 {
    let q = Vec3::ONE - p;
    Vec3::new(
        q.y * q.z * (v[1] - v[0])
            + p.y * q.z * (v[3] - v[2])
            + q.y * p.z * (v[5] - v[4])
            + p.y * p.z * (v[7] - v[6]),
        q.x * q.z * (v[2] - v[0])
            + p.x * q.z * (v[3] - v[1])
            + q.x * p.z * (v[6] - v[4])
            + p.x * p.z * (v[7] - v[5]),
        q.x * q.y * (v[4] - v[0])
            + p.x * q.y * (v[5] - v[1])
            + q.x * p.y * (v[6] - v[2])
            + p.x * p.y * (v[7] - v[3]),
    )
}

/// `place` in `nets.wgsl`, position only: the QEF point about the crossings' mean, kept in the cell.
fn place(v: [f32; 8]) -> Vec3 {
    let mut ata = Mat3::ZERO;
    let (mut atb, mut mass, mut count) = (Vec3::ZERO, Vec3::ZERO, 0.0_f32);
    for k in 0..8 {
        for axis in 0..3 {
            let bit = 1 << axis;
            if k & bit != 0 {
                continue;
            }
            let (a, b) = (v[k], v[k | bit]);
            if (a < 0.0) == (b < 0.0) {
                continue;
            }
            let mut p = Vec3::new((k & 1) as f32, ((k >> 1) & 1) as f32, (k >> 2) as f32);
            p[axis] = a / (a - b);
            let grad = gradient(v, p);
            let n = match grad.dot(grad) > 0.0 {
                true => grad.normalize(),
                false => Vec3::ZERO,
            };
            ata += Mat3::from_cols(n * n.x, n * n.y, n * n.z);
            atb += n * n.dot(p);
            mass += p;
            count += 1.0;
        }
    }
    if count == 0.0 {
        return Vec3::splat(0.5);
    }
    mass /= count;
    let rhs = atb - ata * mass;
    let m = ata + Mat3::from_diagonal(Vec3::splat(QEF_BIAS * count));
    let det = m.determinant();
    let mut y = Vec3::ZERO;
    if det > 1e-12 {
        let adj = Mat3::from_cols(
            m.y_axis.cross(m.z_axis),
            m.z_axis.cross(m.x_axis),
            m.x_axis.cross(m.y_axis),
        );
        y = adj * rhs / det;
    }
    (mass + y).clamp(Vec3::ZERO, Vec3::ONE)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat ground at `height` voxels above sample 0, across the whole tile.
    fn flat(height: f32) -> Vec<f32> {
        (0..TILE * TILE * TILE)
            .map(|i| (i / (TILE * TILE)) as f32 - height)
            .collect()
    }

    #[test]
    fn flat_ground_lies_flat() {
        let mesh = mesh_block(&flat(8.4), 0.5);
        // 16 × 16 z-edges cross, one quad each.
        assert_eq!(mesh.indices.len(), 2 * 16 * 16);
        for p in &mesh.vertices {
            assert!((p.z - (8.4 - 1.0) * 0.5).abs() < 1e-4, "{p}");
        }
    }

    #[test]
    fn air_meshes_nothing() {
        assert!(
            mesh_block(&vec![1.0; TILE * TILE * TILE], 0.5)
                .indices
                .is_empty()
        );
    }
}
