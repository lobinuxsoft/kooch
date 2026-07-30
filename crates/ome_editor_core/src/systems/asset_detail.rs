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

use crate::panels::inspector::{
    AssetDetail, ImageImportInfo, MeshImportInfo, PrefabComponentView, PrefabDetail,
    PrefabEntityView, ResolvedComponent,
};

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
        crate::drag_drop::PREFAB_TYPE_NAME => gather_prefab(guid, resources),
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

/// Resolves a prefab into something the Inspector can draw.
///
/// The document is read from `Assets<SceneDocument>` rather than from disk:
/// that is where edits live until the user saves, so the panel shows what
/// will be spawned rather than what the file still says.
///
/// Resolution happens here, with the registries in hand, so the panel does
/// not need a world — the same split `EntityDisplayInfo` uses.
fn gather_prefab(guid: Guid, resources: &mut Resources) -> Option<AssetDetail> {
    use ome_ecs::scene::SceneDocument;

    // Logged rather than swallowed: the panel's only other state is
    // "Loading asset…", so a load that can never succeed is indistinguishable
    // from one that has not finished. This is how a missing `Assets` store
    // presented as a permanent spinner.
    let Some(handle) = load_handle::<SceneDocument>(guid, resources) else {
        tracing::warn!(target: "ome_editor_core::asset_detail", %guid, "prefab could not be loaded");
        return None;
    };
    let Some(document) = resources
        .get::<Assets<SceneDocument>>()
        .and_then(|assets| assets.get(handle).cloned())
    else {
        tracing::warn!(target: "ome_editor_core::asset_detail", %guid, "prefab loaded but absent from its asset store");
        return None;
    };
    let dirty = resources
        .get::<crate::actions::DirtyPrefabs>()
        .is_some_and(|dirty| dirty.contains(guid));

    let registry = resources.get::<ome_ecs::component::ComponentRegistry>();
    let names = resources.get::<ome_ecs::component::ComponentNames>();

    let entities = document
        .entities
        .iter()
        .enumerate()
        .map(|(index, entity)| PrefabEntityView {
            name: entity.name.clone(),
            index,
            // The root is the entity with no parent — the same rule
            // `SceneDocument::root_index` uses to decide what can be
            // instanced as a unit.
            is_root: !entity
                .components
                .iter()
                .any(|c| short_name(&c.type_name) == "Parent"),
            components: entity
                .components
                .iter()
                .map(|component| PrefabComponentView {
                    short_name: short_name(&component.type_name).to_owned(),
                    type_name: component.type_name.clone(),
                    fields: component.fields.clone(),
                    resolved: registry.as_deref().and_then(|registry| {
                        let type_id = registry.type_id_by_name(&component.type_name)?;
                        Some(ResolvedComponent {
                            type_id,
                            component: names
                                .as_deref()
                                .and_then(|n| n.id(&component.type_name))
                                .unwrap_or(ome_ecs::component::ComponentId::INVALID),
                            field_metas: registry.reflect_field_metas(&type_id),
                        })
                    }),
                })
                .collect(),
        })
        .collect();

    Some(AssetDetail::Prefab(Box::new(PrefabDetail {
        dirty,
        entities,
    })))
}

fn short_name(type_name: &str) -> &str {
    type_name.rsplit("::").next().unwrap_or(type_name)
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
