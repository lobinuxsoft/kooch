//! Single-entity inspector — name editor + field rendering.

use std::any::TypeId;
use std::collections::HashMap;

use glam::Vec3;

use kooch_ecs::component::ComponentId;
use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::{FieldMeta, ReflectValue};

use crate::actions::EditorAction;
use crate::state::{EntityDisplayInfo, EulerCacheKey};

use super::rotation::{draw_quat_with_cache, is_transform_rotation};
use super::widgets::{
    bits_for, choices_for, draw_readonly_value, draw_value_widget, requires_for, AssetCatalogEntry,
    FieldContext,
};
use super::RotationContext;

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
    //
    // The edit leaves as a `SetField` and the value comes back through the
    // world snapshot, which in a remote project is at least a frame later.
    // Rebuilding the box from the snapshot every frame therefore showed
    // the *previous* text for a frame — one character shorter than what
    // egui had just been told. egui clamps the caret to the text it is
    // given, so the caret fell back a place and stayed there: type `abc`
    // and get `cba`, which is what this looked like from the outside.
    //
    // Held in egui's temp store rather than the editor state: it lives
    // exactly as long as the focus does, and it is keyed per entity, so
    // clicking to another entity cannot inherit a half-typed name. The
    // reference picker's search box already works this way.
    // The box is keyed on nothing: it is the same box whichever entity is
    // selected, it never moves, and giving it the entity would rename it
    // on every selection change — the id churn of #641, reintroduced by
    // the fix for the caret. Only the *buffer* is per-entity, so a
    // half-typed name cannot follow the selection to the next entity.
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

/// Which text the name box shows: what is being typed, or what the world
/// says.
///
/// The whole of the caret bug is in this choice. The snapshot is a frame
/// or more behind the keystroke, so preferring it while the field has
/// focus hands egui a string one character shorter than the one it just
/// produced — and egui clamps the caret to the text it is given.
pub(super) fn text_to_show(focused: bool, buffer: Option<String>, snapshot: &str) -> String {
    match focused {
        true => buffer.unwrap_or_else(|| snapshot.to_owned()),
        false => snapshot.to_owned(),
    }
}

/// The field's doc comment, for the Inspector tooltip (#737).
///
/// `""` when the field has no doc comment, or when the component has no
/// static reflection at all — a component parked from a project `.so`
/// reaches the Inspector without `FieldMeta`, so its fields show a
/// tooltip only once the schema carries the string too.
///
/// The alternative was a `#[reflect(tooltip = "...")]` attribute. A
/// second place to write the explanation is a second place for it to go
/// stale, and the stale one is always the one the user reads.
pub(super) fn doc_for(field_metas: Option<&'static [FieldMeta]>, name: &str) -> &'static str {
    field_metas
        .and_then(|metas| metas.iter().find(|m| m.name == name))
        .map(|m| m.doc)
        .unwrap_or("")
}

/// Whether a field's [`FieldCondition`] is met by the component's current
/// values.
///
/// A field with no condition always shows, and a condition naming a field
/// that is not present shows too — a typo in an attribute should look like
/// a mistake, not like a field that silently vanished.
///
/// [`FieldCondition`]: kooch_ecs::reflect::FieldCondition
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
///
/// # Why runs and not a grouping
///
/// Collecting every field of a group together would reorder the
/// author's struct behind their back, and the order of fields in a
/// settings asset is itself information — exposure reads as a triple
/// because aperture, shutter and ISO sit in that order. A run keeps the
/// declaration order and lets a heading appear twice if the struct says
/// so, which is visible and fixable rather than silent.
///
/// The `shown_when` filter is applied here rather than at draw time, so
/// a group whose every field is hidden by the current variant does not
/// leave a heading with nothing under it.
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

/// Reads a reflected value as an `i64`, for comparing against a
/// [`FieldCondition`]'s values. `None` for anything not an integer — a
/// condition on a float or a vector is meaningless, and treating it as
/// unmet would hide the field for good.
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
        // 🔴 A bool IS a discriminant with two values, and leaving it
        // out does not fail loudly: `is_met(None)` reads as SHOWN, so a
        // `shown_when` pointing at a toggle silently never hides
        // anything. The absent-field case is meant to look like a typo;
        // an unsupported TYPE looked like a working rule instead.
        _ => None,
    }
}

/// Renders editable widgets for reflected component fields.
///
/// `euler_cache` lets the Quat path preserve editor-side Euler state to
/// avoid gimbal lock from a per-frame Quat→Euler→Quat round-trip (#202).
/// `rotation_ctx` is only consulted for `Transform.rotation` so the
/// Inspector can toggle Local vs World display (#205). Other Quat fields
/// always edit in local space.
///
/// `entities` is what the reference picker offers as targets.
///
/// # Why it reports rather than applies
///
/// It returns the fields that changed instead of emitting `SetField`
/// actions itself, because what a change *means* depends on what is being
/// inspected: a live entity edits the world, and a prefab edits a document
/// on disk. Both want the same grid, the same pickers and the same
/// `shown_when` rules, and the only way to have one of those is for this
/// function not to decide.
///
/// `entity` is used solely to key the euler-angle cache; a caller with no
/// entity passes a synthetic one.
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
) -> Vec<(String, ReflectValue)> {
    let mut edits = Vec::new();
    // One grid per heading (#830). A single grid for the whole component
    // is what produced the pile the settings asset had become: fourteen
    // rows with no indication of which three belong to the exposure and
    // which five to the shadows.
    //
    // Keyed on the component **and the heading** — see the note in
    // `mod.rs` for why the entity is not part of it. The heading is, and
    // has to be: two grids sharing an id share their column widths, so
    // one long tooltip in the shadows section would resize the exposure
    // section above it.
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
                    // Keyed on the field, not on its position in the
                    // grid. `field_is_shown` hides a variant's unused
                    // parameters, so the row count changes as a collider
                    // switches shape — and with automatic ids that
                    // renames every widget below.
                    //
                    // One scope per cell, not one around the row: a
                    // scope advances the grid's cursor, so wrapping both
                    // would put the label and its editor in the same
                    // column.
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
                            assets: asset_catalog,
                            entities,
                            requires: requires_for(field_metas, name),
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
