//! Inspector panel — component details for selected entities.
//!
//! Split across submodules to keep each file under the project's
//! "no monolíticos" guideline:
//! - [`multi`]: rendering when multiple entities are selected (merged view).
//! - [`single`]: rendering when a single entity is selected (full per-component).
//! - [`rotation`]: gimbal-safe Quat editing with display-mode toggle (#202, #205).
//! - [`widgets`]: per-`ReflectValue` editor widgets and choice dropdowns.

mod asset_view;
mod multi;
mod rotation;
mod single;
mod widgets;

use std::any::TypeId;
use std::collections::{HashMap, HashSet};

use glam::{Quat, Vec3};

use ome_core::Guid;
use ome_ecs::entity::Entity;
use ome_ecs::reflect::{InspectorVisibility, ReflectValue};

use crate::actions::EditorAction;
use crate::drag_drop::DraggedComponent;
use crate::icons;
use crate::state::{
    ComponentDisplayInfo, EntityDisplayInfo, EulerCacheKey, ReflectedTypeInfo, RotationDisplayMode,
};

pub(crate) use asset_view::{AssetDetail, ImageImportInfo, MeshImportInfo};
pub(crate) use widgets::{AssetCatalogEntry, draw_asset_picker};

/// Threshold for considering a cached Euler still in sync with the
/// underlying quaternion. Compared against `|dot(actual, reconstructed)|`
/// since `q` and `-q` represent the same rotation.
pub(super) const EULER_CACHE_EPS: f32 = 1.0e-4;

/// Context for editing the `Transform.rotation` field in either Local
/// or World display mode. The World path uses `self_global` for display
/// and `parent_global` to convert edits back to local space.
#[derive(Clone, Copy)]
pub(super) struct RotationContext {
    pub(super) mode: RotationDisplayMode,
    /// World-space rotation of the entity itself, from `GlobalTransform`.
    pub(super) self_global: Option<Quat>,
    /// Parent's world-space rotation, if the entity has a parent with
    /// a `GlobalTransform`. `None` means treat the parent as identity
    /// (root entity or parent without propagated global transform).
    pub(super) parent_global: Option<Quat>,
}

impl RotationContext {
    pub(super) fn local_only() -> Self {
        Self {
            mode: RotationDisplayMode::Local,
            self_global: None,
            parent_global: None,
        }
    }
}

/// Content of the "Inspector" tab — component details for selected entities.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_inspector_content(
    ui: &mut egui::Ui,
    entities: &[EntityDisplayInfo],
    selected: &[Entity],
    reflected_types: &[ReflectedTypeInfo],
    actions: &mut Vec<EditorAction>,
    euler_cache: &mut HashMap<EulerCacheKey, Vec3>,
    rotation_display_mode: &mut RotationDisplayMode,
    asset_catalog: &[AssetCatalogEntry],
    selected_asset: Option<Guid>,
    asset_detail: Option<&AssetDetail>,
) {
    // Asset selection takes over the Inspector — it serves both entities
    // and assets. When an asset is selected, render its view and return.
    if let Some(guid) = selected_asset
        && let Some(entry) = asset_catalog.iter().find(|e| e.guid == guid)
    {
        asset_view::draw_asset_inspector(ui, entry, asset_detail, asset_catalog, actions);
        return;
    }

    // Evict cache entries for entities that are no longer selected.
    euler_cache.retain(|(entity, _, _, _), _| selected.contains(entity));

    // The Local/World rotation toggle moved to the viewport toolbar
    // (panels/view.rs). It now only renders when a selected entity has
    // a `Transform` component — see `View` panel for the gating logic.

    // Whole inspector area is a drop zone for DraggedComponent. On drop,
    // the component is added to every selected entity. See #209.
    let (_, dropped) = ui.dnd_drop_zone::<DraggedComponent, ()>(egui::Frame::default(), |ui| {
        draw_inspector_body(
            ui,
            entities,
            selected,
            reflected_types,
            actions,
            euler_cache,
            rotation_display_mode,
            asset_catalog,
        )
    });

    if let Some(payload) = dropped {
        for &entity in selected {
            actions.push(EditorAction::AddComponent {
                entity,
                type_id: payload.0,
            });
        }
    }
}

/// Inspector body — everything below the rotation-mode toggle. Split out
/// so it can be wrapped by a `dnd_drop_zone` without disturbing the
/// early-return control flow the old function used.
#[allow(clippy::too_many_arguments)]
fn draw_inspector_body(
    ui: &mut egui::Ui,
    entities: &[EntityDisplayInfo],
    selected: &[Entity],
    reflected_types: &[ReflectedTypeInfo],
    actions: &mut Vec<EditorAction>,
    euler_cache: &mut HashMap<EulerCacheKey, Vec3>,
    rotation_display_mode: &mut RotationDisplayMode,
    asset_catalog: &[AssetCatalogEntry],
) {
    if selected.is_empty() {
        ui.weak("No entity selected");
        return;
    }

    if selected.len() > 1 {
        multi::draw_multi_entity_inspector(
            ui,
            entities,
            selected,
            reflected_types,
            actions,
            asset_catalog,
        );
        return;
    }

    // Single entity selected — show full inspector.
    let entity = selected[0];
    let Some(info) = entities.iter().find(|e| e.entity == entity) else {
        ui.weak("Entity not found (despawned?)");
        return;
    };

    let entity_name = info
        .components
        .iter()
        .find(|c| c.short_name == "Name")
        .and_then(|c| c.fields.as_ref())
        .and_then(|fields| {
            fields.iter().find_map(|(name, val)| {
                if name == "value" {
                    if let ReflectValue::String(s) = val {
                        if !s.is_empty() {
                            return Some(s.clone());
                        }
                    }
                }
                None
            })
        });

    if let Some(name) = &entity_name {
        ui.label(format!(
            "{} {}  ({}:{})",
            icons::CUBE,
            name,
            entity.index(),
            entity.generation()
        ));
    } else {
        ui.label(format!(
            "{} Entity  index: {}  generation: {}",
            icons::CUBE,
            entity.index(),
            entity.generation()
        ));
    }
    ui.separator();

    // Editable name field (separate from component list).
    single::draw_name_editor(ui, entity, info, actions);

    // "Add Component" dropdown.
    let existing: HashSet<TypeId> = info.components.iter().map(|c| c.type_id).collect();
    let available: Vec<&ReflectedTypeInfo> = reflected_types
        .iter()
        .filter(|t| !existing.contains(&t.type_id))
        .collect();

    if !available.is_empty() {
        ui.menu_button(format!("{} Add Component", icons::PLUS), |ui| {
            crate::panels::add_component_menu::draw_categorized(ui, &available, |type_id| {
                actions.push(EditorAction::AddComponent { entity, type_id });
            });
        });
        ui.separator();
    }

    // Filter out Hidden components for display.
    let visible_components: Vec<&ComponentDisplayInfo> = info
        .components
        .iter()
        .filter(|c| c.visibility != InspectorVisibility::Hidden)
        .collect();

    if visible_components.is_empty() {
        ui.weak("(no components)");
        return;
    }

    let rotation_ctx = RotationContext {
        mode: *rotation_display_mode,
        self_global: info.global_rotation,
        parent_global: info.parent_global_rotation,
    };

    egui::ScrollArea::vertical().show(ui, |ui| {
        for comp in &visible_components {
            let is_read_only = comp.visibility == InspectorVisibility::ReadOnly;
            let id = ui.make_persistent_id(format!("comp_{}_{:?}", entity.index(), comp.type_id));
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
                .show_header(ui, |ui| {
                    ui.strong(format!("{} {}", icons::PUZZLE_PIECE, &comp.short_name));
                    // Removal is always available regardless of visibility:
                    // `ReadOnly` gates field edits, not component lifecycle.
                    if ui
                        .small_button(icons::X)
                        .on_hover_text("Remove component")
                        .clicked()
                    {
                        actions.push(EditorAction::RemoveComponent {
                            entity,
                            type_id: comp.type_id,
                        });
                    }
                })
                .body(|ui| {
                    if let Some(fields) = &comp.fields {
                        if fields.is_empty() {
                            ui.weak("(no fields)");
                        } else if is_read_only {
                            single::draw_readonly_fields(
                                ui,
                                entity,
                                comp.type_id,
                                fields,
                                comp.field_metas,
                            );
                        } else {
                            single::draw_reflected_fields(
                                ui,
                                entity,
                                comp.type_id,
                                fields,
                                comp.field_metas,
                                euler_cache,
                                rotation_ctx,
                                actions,
                                asset_catalog,
                            );
                        }
                    } else {
                        ui.weak("(no reflection)");
                    }
                });
        }
    });
}
