//! Resolves the Asset Browser's selected asset into a data snapshot
//! ([`AssetDetail`]) before the egui frame.
//!
//! Runs against `Resources` (needs the `AssetServer` to load-on-demand
//! and the typed `Assets<T>` stores), so it lives on the system side
//! rather than in the panel. The panel only renders the returned
//! snapshot.

use ome_core::Guid;
use ome_core::asset_database::AssetDatabase;
use ome_core::asset_loader::AssetServer;
use ome_core::assets::Assets;
use ome_core::resource::Resources;
use ome_render::material::Material;
use ome_render::meshlet::MeshletMesh;
use ome_render::texture::{Image, ImageFormat};

use crate::panels::asset_browser::{AssetDetail, ImageImportInfo, MeshImportInfo};

/// Builds the detail snapshot for `guid`, loading the asset on demand.
/// Returns `None` when the asset is unknown or fails to load.
pub(crate) fn gather_asset_detail(guid: Guid, resources: &mut Resources) -> Option<AssetDetail> {
    let type_name = resources
        .get::<AssetDatabase>()?
        .entry(guid)?
        .type_name
        .clone()?;

    match type_name.as_str() {
        "ome_render::material::asset::Material" => gather_material(guid, resources),
        "ome_render::meshlet::asset::MeshletMesh" => gather_mesh(guid, resources),
        "ome_render::texture::asset::Image" => gather_image(guid, resources),
        other => Some(AssetDetail::Unknown {
            type_name: other.to_owned(),
        }),
    }
}

fn gather_material(guid: Guid, resources: &mut Resources) -> Option<AssetDetail> {
    let handle = load_handle::<Material>(guid, resources)?;
    let mat = resources.get::<Assets<Material>>()?.get(handle)?.clone();
    Some(AssetDetail::Material(mat))
}

fn gather_mesh(guid: Guid, resources: &mut Resources) -> Option<AssetDetail> {
    let handle = load_handle::<MeshletMesh>(guid, resources)?;
    let meshes = resources.get::<Assets<MeshletMesh>>()?;
    let mesh = meshes.get(handle)?;
    Some(AssetDetail::Mesh(MeshImportInfo {
        vertices: mesh.total_vertex_count(),
        meshlets: mesh.meshlet_count(),
        triangles: mesh.total_triangle_count(),
        aabb_min: mesh.aabb.min,
        aabb_max: mesh.aabb.max,
    }))
}

fn gather_image(guid: Guid, resources: &mut Resources) -> Option<AssetDetail> {
    let handle = load_handle::<Image>(guid, resources)?;
    let images = resources.get::<Assets<Image>>()?;
    let img = images.get(handle)?;
    Some(AssetDetail::Image(ImageImportInfo {
        width: img.width,
        height: img.height,
        format: format_name(img.format),
        bytes: img.byte_count(),
    }))
}

/// Resolves `guid` to a typed handle through the `AssetServer`, loading
/// the asset if it is not already resident. The server is removed and
/// re-inserted around the call because `load_by_guid` needs mutable
/// access to the whole `Resources`.
fn load_handle<T: ome_core::assets::Asset>(
    guid: Guid,
    resources: &mut Resources,
) -> Option<ome_core::assets::Handle<T>> {
    let mut server = resources.remove::<AssetServer>()?;
    let handle = server.load_by_guid::<T>(guid, resources).ok();
    resources.insert(server);
    handle
}

fn format_name(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Rgba8UnormSrgb => "RGBA8 sRGB",
        ImageFormat::Rgba8Unorm => "RGBA8 linear",
    }
}
