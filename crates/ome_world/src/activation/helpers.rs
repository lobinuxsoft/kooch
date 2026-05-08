use glam::{DVec3, IVec3};

use crate::chunk::{BASE_CHUNK_SIZE_METERS, ChunkId};

/// Enumerate every chunk at `lod` whose AABB intersects the sphere
/// `(center, radius)`. Iterates the bounding-box grid with **per-axis
/// early-out**: the squared distance from `center` to the closest
/// point of the current axis slab is accumulated outer → inner, so
/// whole `(y, z)` slices and whole `z` columns are skipped without
/// touching their cells.
///
/// Reduces the cell count tested from the cube bounding box (≈8r³ /
/// chunk³) to roughly the inscribed sphere (≈⁴⁄₃πr³ / chunk³) — about
/// 50 % fewer evaluations in the limit. The exact AABB-vs-sphere test
/// inside the inner loop stays identical, so the result set is
/// byte-identical to the old brute-force version.
pub(super) fn chunks_within_sphere(center: DVec3, radius: f64, lod: u8) -> Vec<ChunkId> {
    let chunk_size = BASE_CHUNK_SIZE_METERS * (1u64 << lod) as f64;
    let radius_sq = radius * radius;

    let min_world = center - DVec3::splat(radius);
    let max_world = center + DVec3::splat(radius);
    let min_idx_x = (min_world.x / chunk_size).floor() as i32;
    let max_idx_x = (max_world.x / chunk_size).ceil() as i32;
    let min_idx_y = (min_world.y / chunk_size).floor() as i32;
    let max_idx_y = (max_world.y / chunk_size).ceil() as i32;
    let min_idx_z = (min_world.z / chunk_size).floor() as i32;
    let max_idx_z = (max_world.z / chunk_size).ceil() as i32;

    let mut out = Vec::new();
    for x in min_idx_x..max_idx_x {
        let cell_min_x = x as f64 * chunk_size;
        let cell_max_x = cell_min_x + chunk_size;
        let dx = center.x - center.x.clamp(cell_min_x, cell_max_x);
        let dx_sq = dx * dx;
        if dx_sq > radius_sq {
            // Whole y-z slice is outside the sphere — skip ~chunk_count² cells.
            continue;
        }

        for y in min_idx_y..max_idx_y {
            let cell_min_y = y as f64 * chunk_size;
            let cell_max_y = cell_min_y + chunk_size;
            let dy = center.y - center.y.clamp(cell_min_y, cell_max_y);
            let xy_sq = dx_sq + dy * dy;
            if xy_sq > radius_sq {
                // Whole z column is outside — skip ~chunk_count cells.
                continue;
            }

            for z in min_idx_z..max_idx_z {
                let cell_min_z = z as f64 * chunk_size;
                let cell_max_z = cell_min_z + chunk_size;
                let dz = center.z - center.z.clamp(cell_min_z, cell_max_z);
                if xy_sq + dz * dz <= radius_sq {
                    out.push(ChunkId::new(IVec3::new(x, y, z), lod));
                }
            }
        }
    }
    out
}

/// Squared distance from the chunk's centre to the closest focus.
/// Lower = higher priority for the load queue (closest pops first).
pub(super) fn chunk_priority(id: ChunkId, focuses: &[(DVec3, u8)]) -> f32 {
    let chunk_size = id.size_meters();
    let centre = DVec3::new(
        id.coords.x as f64 + 0.5,
        id.coords.y as f64 + 0.5,
        id.coords.z as f64 + 0.5,
    ) * chunk_size;

    let mut min_d2 = f64::INFINITY;
    for (pos, _) in focuses {
        let d2 = (centre - *pos).length_squared();
        if d2 < min_d2 {
            min_d2 = d2;
        }
    }
    min_d2 as f32
}
