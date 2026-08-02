//! Resolves the Asset Browser's selected asset into a data snapshot
//! ([`AssetDetail`]) before the egui frame.
//!
//! Runs against `Resources` (needs the `AssetServer` to load-on-demand
//! and the typed `Assets<T>` stores), so it lives on the system side
//! rather than in the panel. The panel only renders the returned
//! snapshot.

use kooch_core::Guid;
use kooch_core::asset_database::AssetDatabase;
use kooch_core::asset_loader::AssetServer;
use kooch_core::assets::Assets;
use kooch_core::resource::Resources;
use kooch_render::material::Material;
use kooch_render::meshlet::MeshletMesh;
use kooch_render::texture::{Image, ImageFormat};

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
        "kooch_render::material::asset::Material" => gather_material(guid, resources),
        "kooch_render::meshlet::asset::MeshletMesh" => gather_mesh(guid, resources),
        "kooch_render::texture::asset::Image" => gather_image(guid, resources),
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
    use kooch_ecs::scene::SceneDocument;

    // Logged rather than swallowed: the panel's only other state is
    // "Loading asset…", so a load that can never succeed is indistinguishable
    // from one that has not finished. This is how a missing `Assets` store
    // presented as a permanent spinner.
    let Some(handle) = load_handle::<SceneDocument>(guid, resources) else {
        tracing::warn!(target: "kooch_editor_core::asset_detail", %guid, "prefab could not be loaded");
        return None;
    };
    let Some(document) = resources
        .get::<Assets<SceneDocument>>()
        .and_then(|assets| assets.get(handle).cloned())
    else {
        tracing::warn!(target: "kooch_editor_core::asset_detail", %guid, "prefab loaded but absent from its asset store");
        return None;
    };
    let dirty = resources
        .get::<crate::actions::DirtyPrefabs>()
        .is_some_and(|dirty| dirty.contains(guid));

    let registry = resources.get::<kooch_ecs::component::ComponentRegistry>();
    let names = resources.get::<kooch_ecs::component::ComponentNames>();
    // The third place a component type can be known from: declared by a
    // project's plugin rather than compiled into the editor. Asking only
    // the reflected registry is what left a project's own component with
    // no fields to edit (#722).
    let dynamic = resources.get::<kooch_ecs::component::DynamicTypeRegistry>();

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
            components: sorted_visible(
                entity,
                registry.as_deref(),
                names.as_deref(),
                dynamic.as_deref(),
            ),
        })
        .collect();

    Some(AssetDetail::Prefab(Box::new(PrefabDetail {
        dirty,
        entities,
    })))
}

/// A prefab entity's components, ordered and filtered the way the entity
/// inspector does it — same rule, same hidden types, so the two panels do
/// not disagree about a list of the same thing.
fn sorted_visible(
    entity: &kooch_ecs::scene::EntityDescription,
    registry: Option<&kooch_ecs::component::ComponentRegistry>,
    names: Option<&kooch_ecs::component::ComponentNames>,
    dynamic: Option<&kooch_ecs::component::DynamicTypeRegistry>,
) -> Vec<PrefabComponentView> {
    let mut components: Vec<PrefabComponentView> = entity
        .components
        .iter()
        .filter_map(|component| {
            let resolved = registry.and_then(|registry| {
                let type_id = registry.type_id_by_name(&component.type_name)?;
                // Hidden means hidden here too. A component the entity
                // inspector never shows appearing in the prefab one
                // reads as the prefab having something extra.
                if registry.reflect_inspector_visibility(&type_id)
                    == Some(kooch_ecs::reflect::InspectorVisibility::Hidden)
                {
                    return None;
                }
                Some(ResolvedComponent {
                    type_id: Some(type_id),
                    component: names
                        .and_then(|n| n.id(&component.type_name))
                        .unwrap_or(kooch_ecs::component::ComponentId::INVALID),
                    field_metas: registry.reflect_field_metas(&type_id),
                })
            });
            // A known-but-hidden type resolves to `None` above, and so
            // does one this binary has no type for — told apart here,
            // because the second must still be shown.
            let known = registry
                .and_then(|r| r.type_id_by_name(&component.type_name))
                .is_some();
            if known && resolved.is_none() {
                return None;
            }

            // Not in the reflected registry, but a project's plugin
            // declared it. Its fields are known and its values are right
            // here in the document, so it renders like any other — which
            // is what `DynamicTypeRegistry`'s own docs already promise.
            //
            // No `TypeId`: this binary has no Rust type for it. Nothing
            // below needs one except the world-space rotation toggle,
            // which only ever applies to the engine's own `Transform`.
            let resolved = resolved.or_else(|| {
                dynamic
                    .filter(|registry| registry.get(&component.type_name).is_some())
                    .map(|_| ResolvedComponent {
                        type_id: None,
                        component: names
                            .and_then(|n| n.id(&component.type_name))
                            .unwrap_or(kooch_ecs::component::ComponentId::INVALID),
                        field_metas: None,
                    })
            });
            Some(PrefabComponentView {
                short_name: short_name(&component.type_name).to_owned(),
                type_name: component.type_name.clone(),
                fields: component.fields.clone(),
                resolved,
            })
        })
        .collect();
    components.sort_by(|a, b| crate::queries::display_order(&a.short_name, &b.short_name));
    components
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
fn load_handle<T: kooch_core::assets::Asset>(
    guid: Guid,
    resources: &mut Resources,
) -> Option<kooch_core::assets::Handle<T>> {
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
