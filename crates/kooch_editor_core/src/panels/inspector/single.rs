//! Single-entity inspector — name editor + field rendering.

use std::any::TypeId;
use std::collections::HashMap;

use glam::Vec3;

use kooch_ecs::component::ComponentId;
use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::{FieldMeta, ReflectValue};

use crate::actions::EditorAction;
use crate::state::{EntityDisplayInfo, EulerCacheKey};

use super::RotationContext;
use super::rotation::{draw_quat_with_cache, is_transform_rotation};
use super::widgets::{
    AssetCatalogEntry, FieldContext, bits_for, choices_for, draw_readonly_value, draw_value_widget,
    fields_for, layer_for, layers_for, range_for, requires_for,
};

/// Draws an editable name field for the Name component (shown above the component list).
pub(super) fn draw_name_editor(
    ui: &mut egui::Ui,
    entity: Entity,
    info: &EntityDisplayInfo,
    actions: &mut Vec<EditorAction>,
) {
    let name_comp = info.components.iter().find(|c| c.short_name == "Name");
    let Some(comp) = name_comp else { return };
    let Some(fields) = comp.fields.values() else {
        return;
    };
    let Some((_, value)) = fields.iter().find(|(n, _)| n == "value") else {
        return;
    };
    let ReflectValue::String(current) = value else {
        return;
    };

    // While the field has focus it owns the text; the snapshot does not.
    let buffer_id = ui.make_persistent_id(("name_edit", entity));
    let field_id = ui.make_persistent_id("name_edit_field");
    let focused = ui.memory(|m| m.has_focus(field_id));

    let buffer = ui.ctx().data(|d| d.get_temp::<String>(buffer_id));
    let mut val = text_to_show(focused, buffer, current);

    ui.horizontal(|ui| {
        ui.label("Name");
        let response = ui.add(egui::TextEdit::singleline(&mut val).id(field_id));
        if response.changed() {
            ui.ctx().data_mut(|d| d.insert_temp(buffer_id, val.clone()));
            actions.push(EditorAction::SetField {
                entity,
                component: comp.component,
                field: "value".to_owned(),
                value: ReflectValue::String(val),
            });
        }
        // Leaving the field hands ownership back to the snapshot, so a
        // rename that the project rejected or altered shows what actually
        // landed rather than what was typed.
        if response.lost_focus() {
            ui.ctx().data_mut(|d| d.remove_temp::<String>(buffer_id));
        }
    });
    ui.separator();
}

/// Which text the name box shows: what is being typed, or what the world says.
pub(super) fn text_to_show(focused: bool, buffer: Option<String>, snapshot: &str) -> String {
    match focused {
        true => buffer.unwrap_or_else(|| snapshot.to_owned()),
        false => snapshot.to_owned(),
    }
}

/// The field's doc comment, for the Inspector tooltip (#737).
pub(super) fn doc_for(field_metas: Option<&'static [FieldMeta]>, name: &str) -> &'static str {
    field_metas
        .and_then(|metas| metas.iter().find(|m| m.name == name))
        .map(|m| m.doc)
        .unwrap_or("")
}

/// Whether a field's [`FieldCondition`](kooch_ecs::reflect::FieldCondition) is met by the
/// component's current values.
pub(super) fn field_is_shown(
    field_metas: Option<&'static [FieldMeta]>,
    name: &str,
    fields: &[(String, ReflectValue)],
) -> bool {
    let Some(condition) = field_metas
        .and_then(|metas| metas.iter().find(|m| m.name == name))
        .and_then(|meta| meta.shown_when)
    else {
        return true;
    };
    let discriminant = fields
        .iter()
        .find(|(n, _)| n == condition.field)
        .and_then(|(_, value)| integer_value(value));
    condition.is_met(discriminant)
}

/// The heading a field is drawn under (#830), or `""` for none.
pub(super) fn group_for(field_metas: Option<&'static [FieldMeta]>, name: &str) -> &'static str {
    field_metas
        .and_then(|metas| metas.iter().find(|m| m.name == name))
        .map(|m| m.group)
        .unwrap_or("")
}

/// Splits the visible fields into consecutive runs that share a heading.
pub(super) fn group_runs<'a>(
    fields: &'a [(String, ReflectValue)],
    field_metas: Option<&'static [FieldMeta]>,
) -> Vec<(&'static str, Vec<&'a (String, ReflectValue)>)> {
    let mut runs: Vec<(&'static str, Vec<&'a (String, ReflectValue)>)> = Vec::new();
    for field in fields {
        if !field_is_shown(field_metas, &field.0, fields) {
            continue;
        }
        let group = group_for(field_metas, &field.0);
        match runs.last_mut() {
            Some((current, members)) if *current == group => members.push(field),
            _ => runs.push((group, vec![field])),
        }
    }
    runs
}

/// Reads a reflected value as an `i64`, for comparing against a [`FieldCondition`]'s values. `None`
/// for anything not an integer — a condition on a float or a vector is meaningless, and treating it
/// as unmet would hide the field for good.
fn integer_value(value: &ReflectValue) -> Option<i64> {
    match value {
        ReflectValue::U8(v) => Some(*v as i64),
        ReflectValue::U16(v) => Some(*v as i64),
        ReflectValue::U32(v) => Some(*v as i64),
        ReflectValue::U64(v) => Some(*v as i64),
        ReflectValue::I8(v) => Some(*v as i64),
        ReflectValue::I16(v) => Some(*v as i64),
        ReflectValue::I32(v) => Some(*v as i64),
        ReflectValue::I64(v) => Some(*v),
        ReflectValue::Bool(v) => Some(i64::from(*v)),
        // 🔴 A bool IS a discriminant with two values, and leaving it out does not fail loudly:
        // `is_met(None)` reads as SHOWN, so a `shown_when` pointing at a toggle silently never
        // hides anything.
        _ => None,
    }
}

/// Renders editable widgets for reflected component fields.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_reflected_fields(
    ui: &mut egui::Ui,
    entity: Entity,
    type_id: Option<TypeId>,
    component: ComponentId,
    fields: &[(String, ReflectValue)],
    field_metas: Option<&'static [FieldMeta]>,
    euler_cache: &mut HashMap<EulerCacheKey, Vec3>,
    rotation_ctx: RotationContext,
    asset_catalog: &[AssetCatalogEntry],
    entities: &[EntityDisplayInfo],
    // The project's layer names, for any field that masks over them (#1218).
    layer_labels: &[String],
) -> Vec<(String, ReflectValue)> {
    let mut edits = Vec::new();
    // One grid per heading (#830). A single grid for the whole component is what produced the pile
    // the settings asset had become: fourteen rows with no indication of which three belong to the
    // exposure and which five to the shadows.
    for (group, members) in group_runs(fields, field_metas) {
        if !group.is_empty() {
            ui.add_space(6.0);
            ui.strong(group);
        }
        egui::Grid::new(format!("fields_{component:?}_{group}"))
            .num_columns(2)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                for (name, value) in members {
                    // Keyed on the field, not on its position in the grid. `field_is_shown` hides a
                    // variant's unused parameters, so the row count changes as a collider switches
                    // shape — and with automatic ids that renames every widget below.
                    ui.push_id(("label", name), |ui| {
                        let label = ui.label(name);
                        let doc = doc_for(field_metas, name);
                        if !doc.is_empty() {
                            label.on_hover_text(doc);
                        }
                    });
                    ui.push_id(name, |ui| {
                        let field = FieldContext {
                            name,
                            choices: choices_for(field_metas, name),
                            bits: bits_for(field_metas, name),
                            layers: layers_for(field_metas, name),
                            layer: layer_for(field_metas, name),
                            layer_labels,
                            assets: asset_catalog,
                            entities,
                            requires: requires_for(field_metas, name),
                            range: range_for(field_metas, name),
                            fields: fields_for(field_metas, name),
                        };
                        let new_value = match value {
                            ReflectValue::Quat(q) => {
                                let ctx = if is_transform_rotation(type_id, name) {
                                    rotation_ctx
                                } else {
                                    RotationContext::local_only()
                                };
                                draw_quat_with_cache(
                                    ui,
                                    entity,
                                    component,
                                    name,
                                    *q,
                                    ctx,
                                    euler_cache,
                                )
                            }
                            _ => draw_value_widget(ui, value, &field),
                        };
                        if let Some(new_value) = new_value {
                            edits.push((name.clone(), new_value));
                        }
                    });
                    ui.end_row();
                }
            });
    }
    edits
}

/// Renders read-only display for component fields.
pub(super) fn draw_readonly_fields(
    ui: &mut egui::Ui,
    component: ComponentId,
    fields: &[(String, ReflectValue)],
    field_metas: Option<&'static [FieldMeta]>,
) {
    for (group, members) in group_runs(fields, field_metas) {
        if !group.is_empty() {
            ui.add_space(6.0);
            ui.strong(group);
        }
        egui::Grid::new(format!("ro_fields_{component:?}_{group}"))
            .num_columns(2)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                for (name, value) in members {
                    ui.push_id(("label", name), |ui| {
                        let label = ui.label(name);
                        let doc = doc_for(field_metas, name);
                        if !doc.is_empty() {
                            label.on_hover_text(doc);
                        }
                    });
                    ui.push_id(name, |ui| {
                        let choices = choices_for(field_metas, name);
                        let bits = bits_for(field_metas, name);
                        draw_readonly_value(ui, value, choices, bits);
                    });
                    ui.end_row();
                }
            });
    }
}

#[cfg(test)]
mod condition_tests;

#[cfg(test)]
mod name_editor_tests;
