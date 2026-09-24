//! Asset inspector — the Inspector panel's view when an *asset* (rather than an entity) is selected
//! in the Asset Browser.

use glam::Vec3;

use kooch_core::Guid;
use kooch_ecs::component::ComponentId;
use kooch_ecs::reflect::{FieldMeta, ReflectValue};
use kooch_render::material::{Material, ParamKind, ParamValue, SHADER_TYPE_NAME, ShaderParam};

use super::prefab_view;
use super::{AssetCatalogEntry, draw_asset_picker};
use crate::actions::{BakeKind, EditorAction};

/// Canonical asset type name the texture pickers filter by.
pub(crate) const IMAGE_TYPE: &str = "kooch_render::texture::asset::Image";

/// Per-frame data snapshot for the selected asset. Cloned out of the
/// asset stores before the egui frame so the panel stays borrow-free.
pub(crate) enum AssetDetail {
    /// Authored material — editable — and what its shader declares.
    Material(Material, MaterialShader),
    /// Baked mesh — read-only import stats.
    Mesh(MeshImportInfo),
    /// Decoded image — read-only import stats.
    Image(ImageImportInfo),
    /// A prefab, editable as the entities it describes.
    Prefab(Box<PrefabDetail>),
    /// Any asset registered with `register_reflected_asset!`, drawn through the same grid
    /// components use (#744).
    Reflected {
        type_name: String,
        fields: Vec<(String, kooch_ecs::reflect::ReflectValue)>,
        field_metas: Option<&'static [kooch_ecs::reflect::FieldMeta]>,
    },
    /// The project's layer names, edited as a table of 32 rows (#1218). Not reflected: a list of
    /// strings is not a field grid.
    Layers(kooch_core::layers::LayerNames),
    /// A typed asset with neither a dedicated view nor reflection.
    Unknown { type_name: String },
}

/// Which fields a material's Inspector shows.
pub(crate) enum MaterialShader {
    /// The engine's PBR surface: the built-in fields.
    Default,
    /// A custom shader: exactly the parameters it declares.
    Declares(Vec<ShaderParam>),
    /// Named, but missing or failing to parse.
    Unavailable,
}

/// A prefab, resolved against this binary's registry and ready to draw.
pub(crate) struct PrefabDetail {
    /// Set while the cached document differs from the file.
    pub dirty: bool,
    pub entities: Vec<PrefabEntityView>,
}

/// One entity described by a prefab.
pub(crate) struct PrefabEntityView {
    pub name: String,
    /// Index into the document — how an edit addresses this entity, since
    /// there is no handle to name it by.
    pub index: usize,
    /// The entity with no `Parent`. Labelled because "which of these is
    /// the thing I dragged in" is the first question a nested prefab
    /// raises.
    pub is_root: bool,
    pub components: Vec<PrefabComponentView>,
}

/// One component on one of a prefab's entities.
pub(crate) struct PrefabComponentView {
    /// Full path — what the document stores, because it outlives the
    /// process that wrote it.
    pub type_name: String,
    pub short_name: String,
    pub fields: Vec<(String, ReflectValue)>,
    /// `None` for a component this binary has no Rust type for. Such a component is parked verbatim
    /// and round-trips intact; showing it as un-editable is the truth, and hiding it would make
    /// saving look like it dropped data.
    pub resolved: Option<ResolvedComponent>,
}

/// What the registry knows about a component named in a document.
#[derive(Clone, Copy)]
pub(crate) struct ResolvedComponent {
    /// `None` for a component this binary has no Rust type for — one a
    /// project's plugin declared. Its fields are still known, from the
    /// schema that plugin published, so it renders like any other.
    pub type_id: Option<std::any::TypeId>,
    pub component: ComponentId,
    pub field_metas: Option<&'static [FieldMeta]>,
}

/// Read-only import statistics for a meshlet mesh.
pub(crate) struct MeshImportInfo {
    pub vertices: u32,
    pub meshlets: u32,
    pub triangles: u32,
    pub aabb_min: Vec3,
    pub aabb_max: Vec3,
    /// Set when this mesh was baked from another one.
    pub baked: Option<BakedFrom>,
}

/// What a baked collision mesh remembers about its source.
pub(crate) struct BakedFrom {
    /// `"hull"` or `"parts"`.
    pub kind: String,
    /// The mesh it was derived from, if the database still knows it.
    pub source: Option<String>,
    /// `true` when the source's bytes no longer hash to what was
    /// recorded — or when the source is gone.
    pub stale: bool,
}

/// Read-only import statistics for a decoded image.
pub(crate) struct ImageImportInfo {
    pub width: u32,
    pub height: u32,
    pub format: &'static str,
    pub bytes: usize,
    /// The `[import]` table's answer, or the engine's default.
    pub import: kooch_render::texture::ImageImport,
    /// How many levels the chain has when it is on — shown because "mipmaps" is an abstraction and
    /// "11 levels" is a fact about this texture, and because a 1x1 image getting one level is the
    /// explanation for a checkbox that appears to do nothing.
    pub levels: u32,
}

/// Renders the Inspector's asset view. `detail` is `None` while the
/// snapshot for a freshly-selected asset is still being resolved (one
/// frame of lag).
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_asset_inspector(
    ui: &mut egui::Ui,
    entry: &AssetCatalogEntry,
    detail: Option<&AssetDetail>,
    catalog: &[AssetCatalogEntry],
    euler_cache: &mut std::collections::HashMap<crate::state::EulerCacheKey, Vec3>,
    entities: &[super::EntityDisplayInfo],
    reflected_types: &[crate::state::ReflectedTypeInfo],
    actions: &mut Vec<EditorAction>,
    // The project's layer names, for any field that masks over them (#1218).
    layer_labels: &[String],
) {
    ui.label(format!(
        "{} {}  [{}]",
        crate::icons::FOLDER_OPEN,
        entry.display_name,
        entry.source.label(),
    ));
    ui.label(entry.path.display().to_string())
        .on_hover_text(format!("guid: {}", entry.guid));
    ui.separator();

    egui::ScrollArea::vertical()
        .id_salt("asset_detail")
        .show(ui, |ui| match detail {
            Some(AssetDetail::Material(mat, shader)) => {
                draw_material_editor(ui, entry.guid, mat, shader, catalog, actions)
            }
            Some(AssetDetail::Prefab(detail)) => prefab_view::draw_prefab_inspector(
                ui,
                entry.guid,
                detail,
                euler_cache,
                catalog,
                entities,
                reflected_types,
                actions,
                layer_labels,
            ),
            Some(AssetDetail::Mesh(info)) => draw_mesh_import(ui, entry.guid, info, actions),
            Some(AssetDetail::Image(info)) => draw_image_import(ui, entry.guid, info, actions),
            Some(AssetDetail::Reflected {
                type_name,
                fields,
                field_metas,
            }) => draw_reflected_asset(
                ui,
                entry.guid,
                type_name,
                fields,
                *field_metas,
                euler_cache,
                catalog,
                entities,
                actions,
                layer_labels,
            ),
            Some(AssetDetail::Layers(names)) => draw_layers(ui, entry.guid, names, actions),
            Some(AssetDetail::Unknown { type_name }) => {
                ui.weak(format!("No import settings for {type_name}."));
            }
            None => {
                ui.weak("Loading asset…");
            }
        });
}

fn draw_mesh_import(
    ui: &mut egui::Ui,
    guid: Guid,
    info: &MeshImportInfo,
    actions: &mut Vec<EditorAction>,
) {
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

    if let Some(baked) = &info.baked {
        ui.add_space(8.0);
        draw_baked_origin(ui, baked);
    }

    ui.add_space(8.0);
    draw_collider_bake(ui, guid, actions);
}

/// Where a baked collision mesh came from, and whether it is behind.
fn draw_baked_origin(ui: &mut egui::Ui, baked: &BakedFrom) {
    ui.weak(format!("Baked collision ({})", baked.kind));
    match &baked.source {
        Some(source) => {
            ui.label(format!("from {source}"));
        }
        None => {
            ui.label("from a mesh the asset database no longer knows");
        }
    }
    if baked.stale {
        // Amber, the same as the dirty-scene marker and a switched-off
        // system: something here does not match what it claims to.
        ui.colored_label(
            egui::Color32::from_rgb(210, 150, 60),
            "The source changed since this was baked — re-bake it",
        );
    }
}

/// The two collision meshes this mesh can be baked into.
fn draw_collider_bake(ui: &mut egui::Ui, guid: Guid, actions: &mut Vec<EditorAction>) {
    ui.weak("Collision mesh");
    let max_faces = ui.data_mut(|d| *d.get_temp_mut_or(BAKE_FACES_ID.with(guid), 0u32));

    let mut bake = |ui: &mut egui::Ui, kind: BakeKind, label: &str, hint: &str, enabled: bool| {
        if ui
            .add_enabled(enabled, egui::Button::new(label))
            .on_hover_text(hint)
            .on_disabled_hover_text("Set a face budget below, or the result is a copy")
            .clicked()
        {
            actions.push(EditorAction::BakeCollider {
                source: guid,
                kind,
                max_faces,
            });
        }
    };

    ui.horizontal(|ui| {
        bake(
            ui,
            BakeKind::Hull,
            "Create hull mesh",
            "One convex hull, written to assets/collision in this project",
            true,
        );
        bake(
            ui,
            BakeKind::Parts,
            "Create convex parts",
            "Decomposes a concave mesh into convex pieces. Slow — seconds — which is \
             exactly why the result is a file",
            true,
        );
    });
    ui.horizontal(|ui| {
        // Needs a budget by construction: decimating to "no limit" writes
        // the same triangles back out under a new GUID.
        bake(
            ui,
            BakeKind::Mesh,
            "Create simplified mesh",
            "The same triangles, decimated to the budget. For static level \
             geometry — and the only bake that MOVES the surface, so check it",
            max_faces > 0,
        );
    });

    ui.horizontal(|ui| {
        let mut faces = max_faces;
        ui.add(
            egui::DragValue::new(&mut faces)
                .speed(4.0)
                .range(0..=65536)
                .prefix("max faces: "),
        )
        .on_hover_text("0 keeps the exact hull. A budget simplifies, then re-hulls to stay convex");
        if faces != max_faces {
            ui.data_mut(|d| d.insert_temp(BAKE_FACES_ID.with(guid), faces));
        }
        if faces == 0 {
            ui.weak("exact");
        }
    });
}

/// Where the face budget lives between frames.
const BAKE_FACES_ID: egui::Id = egui::Id::NULL;

fn draw_image_import(
    ui: &mut egui::Ui,
    guid: Guid,
    info: &ImageImportInfo,
    actions: &mut Vec<EditorAction>,
) {
    ui.weak("Import settings");
    let mut import = info.import;
    egui::Grid::new("image_import")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label("Mipmaps");
            let response = ui.checkbox(&mut import.mipmaps, "");
            response.on_hover_text(
                "Pre-filtered smaller copies, sampled as a surface tilts away from the \
                 camera. Off is for textures read at their own scale — a UI atlas, a \
                 lookup table — where the smaller copies are memory spent to make a 1:1 \
                 sample blurrier.",
            );
            ui.end_row();

            kv(ui, "Size", &format!("{} × {}", info.width, info.height));
            kv(ui, "Format", info.format);
            kv(ui, "Bytes", &info.bytes.to_string());
            kv(
                ui,
                "Levels",
                &if info.import.mipmaps {
                    info.levels.to_string()
                } else {
                    "1".to_owned()
                },
            );
        });
    if import != info.import {
        actions.push(EditorAction::SetImageImport { guid, import });
    }
}

/// One `label: value` grid row.
fn kv(ui: &mut egui::Ui, key: &str, value: &str) {
    ui.label(key);
    ui.label(value);
    ui.end_row();
}

/// Any reflected asset, drawn with the component grid (#744).
#[allow(clippy::too_many_arguments)]
fn draw_reflected_asset(
    ui: &mut egui::Ui,
    guid: Guid,
    type_name: &str,
    fields: &[(String, kooch_ecs::reflect::ReflectValue)],
    field_metas: Option<&'static [kooch_ecs::reflect::FieldMeta]>,
    euler_cache: &mut std::collections::HashMap<super::EulerCacheKey, glam::Vec3>,
    catalog: &[AssetCatalogEntry],
    entities: &[crate::state::EntityDisplayInfo],
    actions: &mut Vec<EditorAction>,
    // The project's layer names, for any field that masks over them (#1218).
    layer_labels: &[String],
) {
    let short = type_name.rsplit("::").next().unwrap_or(type_name);
    ui.label(short);
    ui.separator();

    if fields.is_empty() {
        ui.weak("(no fields)");
        return;
    }

    let bits = guid.as_uuid().as_u128();
    let synthetic_entity = kooch_ecs::entity::Entity::new(bits as u32, ASSET_PSEUDO_GENERATION);
    let synthetic_component = kooch_ecs::component::ComponentId((bits >> 64) as u32);

    let edits = super::single::draw_reflected_fields(
        ui,
        synthetic_entity,
        None,
        synthetic_component,
        fields,
        field_metas,
        euler_cache,
        // An asset has no world transform to display a rotation against.
        super::RotationContext::local_only(),
        catalog,
        entities,
        layer_labels,
    );

    // One write per gesture, not per frame. A slider reports a change every frame it is dragged;
    // persisting each one writes the file, reads it back and round-trips to the running project —
    // 29 times for one drag, measured in #728.
    let commit = !ui.ctx().input(|i| i.pointer.any_down());
    for (field, value) in edits {
        actions.push(EditorAction::EditAssetField {
            guid,
            field,
            value,
            commit,
        });
    }
}

/// The project's layer table: one row per bit, named or not. What a mask's checklist reads.
pub(super) fn draw_layers(
    ui: &mut egui::Ui,
    guid: Guid,
    names: &kooch_core::layers::LayerNames,
    actions: &mut Vec<EditorAction>,
) {
    ui.label("Every renderer, camera, light and collider masks over these.");
    ui.separator();
    egui::Grid::new("layer_names")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            for index in 0..kooch_core::layers::LAYER_COUNT {
                ui.label(format!("{index}"));
                // 🔴 What is being typed lives in the widget's own memory until the field is let
                // go. This table is rebuilt from the file every frame, and a buffer rebuilt with it
                // loses the keystroke that was just typed — the row would never change.
                let id = ui.make_persistent_id(("layer_name", index));
                let mut name = ui
                    .data_mut(|data| data.get_temp::<String>(id))
                    .unwrap_or_else(|| names.label(index));
                // Bit 0 is where everything starts, and a project that renames it renames the
                // default every new renderer lands in — allowed, and worth seeing.
                let response = ui.add(
                    egui::TextEdit::singleline(&mut name)
                        .id(id)
                        .desired_width(f32::INFINITY),
                );
                // 🔴 Asked BEFORE the closure: `has_focus` reads the same memory `data_mut`
                // holds, and asking inside it waits for a lock the asking itself owns.
                let typing = response.has_focus();
                ui.data_mut(|data| {
                    if typing {
                        data.insert_temp(id, name.clone());
                    } else {
                        data.remove_temp::<String>(id);
                    }
                });
                // One write per gesture: the field reports an edit every frame it has focus.
                if response.lost_focus() && name != names.label(index) {
                    actions.push(EditorAction::RenameLayer {
                        guid: Some(guid),
                        index,
                        name,
                    });
                }
                ui.end_row();
            }
        });

    ui.separator();
    draw_collision_matrix(ui, guid, names, actions);
}

/// Where each pair's box sits: one per pair, upper triangle only, in the staircase's own order.
///
/// 🔴 Geometry apart from drawing: a layout only an eye can check is a layout nothing can test, and
/// the rule that matters — one box per pair — is arithmetic.
pub(super) fn matrix_cells(
    origin: egui::Pos2,
    used: &[usize],
    names_width: f32,
    cell: f32,
    header: f32,
) -> Vec<(usize, usize, egui::Rect)> {
    let columns: Vec<usize> = used.iter().rev().copied().collect();
    let mut cells = Vec::new();
    for (row_at, &row) in used.iter().enumerate() {
        for (column_at, &column) in columns.iter().enumerate() {
            if column < row {
                continue;
            }
            let at = egui::pos2(
                origin.x + names_width + cell * column_at as f32,
                origin.y + header + cell * row_at as f32,
            );
            cells.push((
                row,
                column,
                egui::Rect::from_min_size(at, egui::vec2(cell, cell)),
            ));
        }
    }
    cells
}

/// The project's collision matrix: which layers meet which, in one place.
///
/// 🔴 Only the named layers are drawn. Thirty-two rows of `Layer 17` is a wall nobody reads, and a
/// project names the layers it uses — the rest keep colliding with everything, which is what they
/// did before the table existed (#1302).
///
/// Laid out as Unity's: the column names stand on end above a staircase, so one pair is one box and
/// a row is followed across to where its column comes down.
pub(super) fn draw_collision_matrix(
    ui: &mut egui::Ui,
    guid: Guid,
    names: &kooch_core::layers::LayerNames,
    actions: &mut Vec<EditorAction>,
) {
    /// Side of one box, and the pitch of the staircase.
    const CELL: f32 = 20.0;
    /// Room per character of a standing name, and the floor under it: the header is as tall as the
    /// longest name needs, or a name is drawn into the grid and cut off.
    const PER_CHAR: f32 = 7.5;

    let used: Vec<usize> = (0..kooch_core::layers::LAYER_COUNT)
        .filter(|&index| {
            index == 0
                || names
                    .names
                    .get(index)
                    .is_some_and(|name| !name.trim().is_empty())
        })
        .collect();
    if used.is_empty() {
        return;
    }

    ui.label("Which layers collide. Untick a pair and nothing on those two meets, anywhere.");
    ui.add_space(4.0);

    let columns: Vec<usize> = used.iter().rev().copied().collect();
    // Measured by character rather than laid out: the size only reserves room, and a layout wants
    // the font lock this frame already holds.
    let longest = used
        .iter()
        .map(|&index| names.label(index).chars().count() as f32 * PER_CHAR)
        .fold(0.0_f32, f32::max);
    let names_width = longest + 10.0;
    let header = longest + 8.0;
    let size = egui::vec2(
        names_width + CELL * columns.len() as f32,
        header + CELL * used.len() as f32,
    );
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let text_colour = ui.visuals().text_color();
    let line = egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color);

    for (at, &column) in columns.iter().enumerate() {
        let x = rect.left() + names_width + CELL * at as f32 + CELL * 0.5;
        let galley = painter.layout_no_wrap(
            names.label(column),
            egui::FontId::proportional(12.0),
            text_colour,
        );
        // Anchored at the bottom of the header and drawn upwards, so every name ends just above
        // its own column however long it is.
        let mut standing = egui::epaint::TextShape::new(
            egui::pos2(x + 6.0, rect.top() + header - 4.0),
            galley,
            text_colour,
        );
        standing.angle = -std::f32::consts::FRAC_PI_2;
        painter.add(standing);
    }

    for (row_at, &row) in used.iter().enumerate() {
        let y = rect.top() + header + CELL * row_at as f32;
        painter.text(
            egui::pos2(rect.left() + names_width - 6.0, y + CELL * 0.5),
            egui::Align2::RIGHT_CENTER,
            names.label(row),
            egui::FontId::proportional(12.0),
            text_colour,
        );
        // A rule under each row, so a row is followed across without counting boxes.
        painter.line_segment(
            [
                egui::pos2(rect.left() + names_width, y + CELL),
                egui::pos2(
                    rect.left() + names_width + CELL * (columns.len() - row_at) as f32,
                    y + CELL,
                ),
            ],
            line,
        );
    }

    for (row, column, cell) in matrix_cells(rect.min, &used, names_width, CELL, header) {
        painter.line_segment(
            [
                egui::pos2(cell.left(), cell.top()),
                egui::pos2(cell.left(), cell.bottom()),
            ],
            line,
        );
        let mut collide = names.collide(row, column);
        let response = ui.put(cell, egui::Checkbox::without_text(&mut collide));
        if response.changed() {
            actions.push(EditorAction::SetLayerPair {
                guid: Some(guid),
                a: row,
                b: column,
                collide,
            });
        }
        response.on_hover_text(format!("{} × {}", names.label(row), names.label(column)));
    }

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        for (label, collide) in [("Disable All", false), ("Enable All", true)] {
            if ui.button(label).clicked() {
                for &row in &used {
                    for &column in &used {
                        if column >= row {
                            actions.push(EditorAction::SetLayerPair {
                                guid: Some(guid),
                                a: row,
                                b: column,
                                collide,
                            });
                        }
                    }
                }
            }
        }
    });
}

/// Generation no live entity carries, and distinct from the prefab
/// inspector\'s. Keeps asset and prefab euler-cache entries apart when
/// their synthetic indices happen to collide.
const ASSET_PSEUDO_GENERATION: u32 = u32::MAX - 1;

mod material;

use material::*;
