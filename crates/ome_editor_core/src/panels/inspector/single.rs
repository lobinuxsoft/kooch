//! Single-entity inspector — name editor + field rendering.

use std::any::TypeId;
use std::collections::HashMap;

use glam::Vec3;

use ome_ecs::entity::Entity;
use ome_ecs::reflect::{FieldMeta, ReflectValue};

use crate::actions::EditorAction;
use crate::state::{EntityDisplayInfo, EulerCacheKey};

use super::RotationContext;
use super::rotation::{draw_quat_with_cache, is_transform_rotation};
use super::widgets::{AssetCatalogEntry, choices_for, draw_readonly_value, draw_value_widget};

/// Draws an editable name field for the Name component (shown above the component list).
pub(super) fn draw_name_editor(
    ui: &mut egui::Ui,
    entity: Entity,
    info: &EntityDisplayInfo,
    actions: &mut Vec<EditorAction>,
) {
    let name_comp = info.components.iter().find(|c| c.short_name == "Name");
    let Some(comp) = name_comp else { return };
    let Some(fields) = &comp.fields else { return };
    let Some((_, value)) = fields.iter().find(|(n, _)| n == "value") else {
        return;
    };
    let ReflectValue::String(current) = value else {
        return;
    };

    let mut val = current.clone();
    ui.horizontal(|ui| {
        ui.label("Name");
        if ui.text_edit_singleline(&mut val).changed() {
            actions.push(EditorAction::SetField {
                entity,
                type_id: comp.type_id,
                field: "value".to_owned(),
                value: ReflectValue::String(val),
            });
        }
    });
    ui.separator();
}

/// Renders editable widgets for reflected component fields.
///
/// `euler_cache` lets the Quat path preserve editor-side Euler state to
/// avoid gimbal lock from a per-frame Quat→Euler→Quat round-trip (#202).
/// `rotation_ctx` is only consulted for `Transform.rotation` so the
/// Inspector can toggle Local vs World display (#205). Other Quat fields
/// always edit in local space.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_reflected_fields(
    ui: &mut egui::Ui,
    entity: Entity,
    type_id: TypeId,
    fields: &[(String, ReflectValue)],
    field_metas: Option<&'static [FieldMeta]>,
    euler_cache: &mut HashMap<EulerCacheKey, Vec3>,
    rotation_ctx: RotationContext,
    actions: &mut Vec<EditorAction>,
    asset_catalog: &[AssetCatalogEntry],
) {
    egui::Grid::new(format!("fields_{:?}_{}", type_id, entity.index()))
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            for (name, value) in fields {
                ui.label(name);
                let choices = choices_for(field_metas, name);
                let new_value = match value {
                    ReflectValue::Quat(q) => {
                        let ctx = if is_transform_rotation(type_id, name) {
                            rotation_ctx
                        } else {
                            RotationContext::local_only()
                        };
                        draw_quat_with_cache(ui, entity, type_id, name, *q, ctx, euler_cache)
                    }
                    _ => draw_value_widget(ui, value, name, choices, asset_catalog),
                };
                if let Some(new_value) = new_value {
                    actions.push(EditorAction::SetField {
                        entity,
                        type_id,
                        field: name.clone(),
                        value: new_value,
                    });
                }
                ui.end_row();
            }
        });
}

/// Renders read-only display for component fields.
pub(super) fn draw_readonly_fields(
    ui: &mut egui::Ui,
    entity: Entity,
    type_id: TypeId,
    fields: &[(String, ReflectValue)],
    field_metas: Option<&'static [FieldMeta]>,
) {
    egui::Grid::new(format!("ro_fields_{:?}_{}", type_id, entity.index()))
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            for (name, value) in fields {
                ui.label(name);
                let choices = choices_for(field_metas, name);
                draw_readonly_value(ui, value, choices);
                ui.end_row();
            }
        });
}
