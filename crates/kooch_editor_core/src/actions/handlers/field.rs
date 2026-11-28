//! Baking a terrain field (#709): a mesh's signed distances, or an image's heights, into a
//! `.kfield` under `assets/terrain`.

use std::path::{Path, PathBuf};

use glam::Vec3;
use kooch_core::Guid;
use kooch_core::asset_loader::asset_written;
use kooch_core::asset_meta::{self, AssetMeta};
use kooch_core::resource::Resources;
use kooch_render::mesh::parse_mesh_bytes_full;
use kooch_terrain::{FIELD_EXTENSION, Field};

use super::collider::{hash_of, read_source};
use crate::project_state::ProjectState;

const OUT_DIR: &str = "terrain";
const ASSET_TYPE: &str = "kooch_terrain::field::Field";
/// Room around the mesh, over its largest half-extent, so the field still reaches outside it.
const PADDING: f32 = 0.1;
const MOST_RESOLUTION: u32 = 256;
const MOST_HEIGHTS: u32 = 4096;

pub(super) fn handle_bake_sdf(resources: &mut Resources, source: Guid, resolution: u32) {
    let Some((path, bytes)) = read_source(resources, source) else {
        return;
    };
    let mesh = match parse_mesh_bytes_full(&bytes, 1.0, path.parent()) {
        Ok(mesh) => mesh,
        Err(error) => {
            tracing::warn!(guid = %source, %error, "Create SDF field: the mesh will not parse");
            return;
        }
    };
    let positions: Vec<Vec3> = mesh
        .vertices
        .iter()
        .map(|v| Vec3::from(v.position))
        .collect();
    let triangles: Vec<[u32; 3]> = mesh
        .indices
        .chunks_exact(3)
        .map(|t| [t[0], t[1], t[2]])
        .collect();
    let (Some(min), Some(max)) = (
        positions.iter().copied().reduce(Vec3::min),
        positions.iter().copied().reduce(Vec3::max),
    ) else {
        tracing::warn!(guid = %source, "Create SDF field: the mesh has no vertices");
        return;
    };
    let centre = (min + max) * 0.5;
    let largest = ((max - min) * 0.5).max_element().max(1e-6);
    let half = (max - min) * 0.5 + Vec3::splat(largest * PADDING);
    let unit = half.max_element();
    let extent = half / unit;
    let resolution = resolution.clamp(8, MOST_RESOLUTION) as f32;
    let size = extent
        .to_array()
        .map(|e| ((resolution * e).round() as u32).max(2));
    let mut points = Vec::with_capacity(size.iter().product::<u32>() as usize);
    for z in 0..size[2] {
        for y in 0..size[1] {
            for x in 0..size[0] {
                let t = Vec3::new(x as f32, y as f32, z as f32)
                    / Vec3::from_array(size.map(|n| (n - 1) as f32));
                points.push(centre - half + 2.0 * half * t);
            }
        }
    }
    let started = std::time::Instant::now();
    let Some(distances) = kooch_physics::signed_distances(&positions, &triangles, &points) else {
        tracing::warn!(guid = %source, "Create SDF field: parry refused the triangles");
        return;
    };
    let normal: Vec<f32> = distances.iter().map(|d| d / unit).collect();
    let Some(field) = Field::from_distances(size, extent.to_array(), &normal) else {
        tracing::warn!(guid = %source, "Create SDF field: the distances do not make a field");
        return;
    };
    tracing::info!(guid = %source, ?size, seconds = started.elapsed().as_secs_f32(), "SDF baked");
    write(resources, &path, "sdf", source, hash_of(&bytes), &field);
}

pub(super) fn handle_bake_height(resources: &mut Resources, source: Guid, resolution: u32) {
    let Some((path, bytes)) = read_source(resources, source) else {
        return;
    };
    let field = match height_field(&bytes, resolution) {
        Ok(field) => field,
        Err(error) => {
            tracing::warn!(guid = %source, %error, "Create height field");
            return;
        }
    };
    write(resources, &path, "height", source, hash_of(&bytes), &field);
}

/// An image's brightness as a height field, averaged down to `resolution` texels along its longer
/// side. What **Create height field** bakes, for tools that bake without the editor.
pub fn height_field(bytes: &[u8], resolution: u32) -> Result<Field, String> {
    let image = image::load_from_memory(bytes)
        .map_err(|error| format!("the image will not decode: {error}"))?
        .to_luma32f();
    let (width, depth) = image.dimensions();
    let most = resolution.clamp(16, MOST_HEIGHTS);
    Field::from_heights_at(width, depth, image.as_raw(), most)
        .ok_or_else(|| format!("a {width}×{depth} image is too large, or not finite"))
}

fn write(
    resources: &mut Resources,
    path: &Path,
    suffix: &str,
    source: Guid,
    hash: u64,
    field: &Field,
) {
    let Some(dir) = project_dir(resources) else {
        tracing::warn!("Create field: no project is open, so there is nowhere to write it");
        return;
    };
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("field");
    let out = dir.join(format!("{stem}_{suffix}.{FIELD_EXTENSION}"));
    if let Err(error) =
        std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&out, field.to_bytes()))
    {
        tracing::warn!(path = %out.display(), %error, "Create field: cannot write the file");
        return;
    }
    // An existing sidecar keeps its GUID, so every stamp already pointing here still does.
    let mut meta = asset_meta::read_meta(&out).unwrap_or_else(|_| AssetMeta::with_type(ASSET_TYPE));
    meta.asset_type = Some(ASSET_TYPE.to_owned());
    let mut import = meta.import.take().unwrap_or_default();
    import.insert("source_guid".into(), source.to_string().into());
    import.insert("source_hash".into(), format!("{hash:016x}").into());
    meta.import = Some(import);
    if let Err(error) = asset_meta::write_meta(&out, &meta) {
        tracing::warn!(path = %out.display(), %error, "field written without a sidecar");
    }
    asset_written(&out, resources);
    tracing::info!(path = %out.display(), "field written; point a Field stamp at it");
}

/// `<project>/assets/terrain`.
fn project_dir(resources: &Resources) -> Option<PathBuf> {
    let state = resources.get::<ProjectState>()?;
    let root = state.active_project.as_ref()?.root_path.clone();
    Some(root.join("assets").join(OUT_DIR))
}
