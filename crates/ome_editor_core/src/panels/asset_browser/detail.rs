//! Detail / editor pane for the asset selected in the Asset Browser.
//!
//! The panel shows read-only *import settings* for assets that are
//! baked at import time (meshes, textures) and editable *input
//! parameters* for authored assets (materials: colours, scalars,
//! texture slots). Material edits are emitted as
//! [`EditorAction::EditMaterial`], which writes the change back to the
//! asset's `.ron`.
//!
//! [`AssetDetail`] is a per-frame snapshot resolved by
//! [`crate::systems::asset_detail::gather_asset_detail`] from the
//! selected asset's data, so this module never touches `Resources`.

use glam::Vec3;

use ome_core::Guid;
use ome_ecs::reflect::ReflectValue;
use ome_render::material::Material;

use crate::actions::EditorAction;
use crate::panels::inspector::{AssetCatalogEntry, draw_asset_picker};

/// Canonical asset type name the texture pickers filter by.
const IMAGE_TYPE: &str = "ome_render::texture::asset::Image";

/// Per-frame data snapshot for the selected asset. Cloned out of the
/// asset stores before the egui frame so the panel stays borrow-free.
pub(crate) enum AssetDetail {
    /// Authored material — editable.
    Material(Material),
    /// Baked mesh — read-only import stats.
    Mesh(MeshImportInfo),
    /// Decoded image — read-only import stats.
    Image(ImageImportInfo),
    /// A typed asset with no dedicated detail view yet.
    Unknown { type_name: String },
}

/// Read-only import statistics for a meshlet mesh.
pub(crate) struct MeshImportInfo {
    pub vertices: u32,
    pub meshlets: u32,
    pub triangles: u32,
    pub aabb_min: Vec3,
    pub aabb_max: Vec3,
}

/// Read-only import statistics for a decoded image.
pub(crate) struct ImageImportInfo {
    pub width: u32,
    pub height: u32,
    pub format: &'static str,
    pub bytes: usize,
}

/// Renders the detail pane for the selected asset. `detail` is `None`
/// while the snapshot for a freshly-selected asset is still being
/// resolved (one frame of lag).
pub(crate) fn draw_detail(
    ui: &mut egui::Ui,
    entry: &AssetCatalogEntry,
    detail: Option<&AssetDetail>,
    catalog: &[AssetCatalogEntry],
    actions: &mut Vec<EditorAction>,
) {
    ui.add_space(2.0);
    ui.strong(&entry.display_name);
    ui.label(entry.path.display().to_string())
        .on_hover_text(format!("guid: {}", entry.guid));
    ui.separator();

    match detail {
        Some(AssetDetail::Material(mat)) => {
            draw_material_editor(ui, entry.guid, mat, catalog, actions)
        }
        Some(AssetDetail::Mesh(info)) => draw_mesh_import(ui, info),
        Some(AssetDetail::Image(info)) => draw_image_import(ui, info),
        Some(AssetDetail::Unknown { type_name }) => {
            ui.weak(format!("No import settings for {type_name}."));
        }
        None => {
            ui.weak("Loading asset…");
        }
    }
    ui.add_space(2.0);
}

/// Editable material parameters. Emits a single `EditMaterial` action
/// carrying the full edited material whenever any widget changes this
/// frame. Persisting / live GPU sync is the action handler's job.
fn draw_material_editor(
    ui: &mut egui::Ui,
    guid: Guid,
    mat: &Material,
    catalog: &[AssetCatalogEntry],
    actions: &mut Vec<EditorAction>,
) {
    let mut edited = mat.clone();
    let mut changed = false;

    egui::Grid::new(("material_editor", guid))
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label("Base color");
            changed |= ui
                .color_edit_button_rgba_unmultiplied(&mut edited.base_color)
                .changed();
            ui.end_row();

            ui.label("Metallic");
            changed |= ui
                .add(egui::Slider::new(&mut edited.metallic, 0.0..=1.0))
                .changed();
            ui.end_row();

            ui.label("Roughness");
            changed |= ui
                .add(egui::Slider::new(&mut edited.roughness, 0.0..=1.0))
                .changed();
            ui.end_row();

            ui.label("Emissive");
            changed |= ui
                .add(
                    egui::DragValue::new(&mut edited.emissive)
                        .speed(0.01)
                        .range(0.0..=100.0),
                )
                .changed();
            ui.end_row();
        });

    ui.separator();
    ui.label("Textures");
    changed |= texture_row(ui, "Albedo", &mut edited.albedo, catalog);
    changed |= texture_row(ui, "Normal", &mut edited.normal, catalog);
    changed |= texture_row(ui, "Metal/Rough", &mut edited.metal_roughness, catalog);

    if changed {
        actions.push(EditorAction::EditMaterial {
            guid,
            material: edited,
        });
    }
}

/// One texture slot row backed by the shared typed asset picker. Returns
/// `true` when the assignment changed this frame.
fn texture_row(
    ui: &mut egui::Ui,
    label: &str,
    field: &mut Option<Guid>,
    catalog: &[AssetCatalogEntry],
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        if let Some(ReflectValue::AssetRef { guid, .. }) =
            draw_asset_picker(ui, *field, IMAGE_TYPE, catalog)
        {
            *field = guid;
            changed = true;
        }
    });
    changed
}

fn draw_mesh_import(ui: &mut egui::Ui, info: &MeshImportInfo) {
    ui.weak("Import settings (read-only)");
    egui::Grid::new("mesh_import")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            kv(ui, "Vertices", &info.vertices.to_string());
            kv(ui, "Meshlets", &info.meshlets.to_string());
            kv(ui, "Triangles", &info.triangles.to_string());
            kv(
                ui,
                "AABB min",
                &format!(
                    "{:.2}, {:.2}, {:.2}",
                    info.aabb_min.x, info.aabb_min.y, info.aabb_min.z
                ),
            );
            kv(
                ui,
                "AABB max",
                &format!(
                    "{:.2}, {:.2}, {:.2}",
                    info.aabb_max.x, info.aabb_max.y, info.aabb_max.z
                ),
            );
        });
}

fn draw_image_import(ui: &mut egui::Ui, info: &ImageImportInfo) {
    ui.weak("Import settings (read-only)");
    egui::Grid::new("image_import")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            kv(ui, "Size", &format!("{} × {}", info.width, info.height));
            kv(ui, "Format", info.format);
            kv(ui, "Bytes", &info.bytes.to_string());
        });
}

/// One `label: value` grid row.
fn kv(ui: &mut egui::Ui, key: &str, value: &str) {
    ui.label(key);
    ui.label(value);
    ui.end_row();
}
