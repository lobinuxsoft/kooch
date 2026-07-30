//! Single-entity inspector — name editor + field rendering.

use std::any::TypeId;
use std::collections::HashMap;

use glam::Vec3;

use ome_ecs::component::ComponentId;
use ome_ecs::entity::Entity;
use ome_ecs::reflect::{FieldMeta, ReflectValue};

use crate::actions::EditorAction;
use crate::state::{EntityDisplayInfo, EulerCacheKey};

use super::RotationContext;
use super::rotation::{draw_quat_with_cache, is_transform_rotation};
use super::widgets::{
    AssetCatalogEntry, FieldContext, bits_for, choices_for, draw_readonly_value, draw_value_widget,
    requires_for,
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
    let Some(fields) = &comp.fields else { return };
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

/// Whether a field's [`FieldCondition`] is met by the component's current
/// values.
///
/// A field with no condition always shows, and a condition naming a field
/// that is not present shows too — a typo in an attribute should look like
/// a mistake, not like a field that silently vanished.
///
/// [`FieldCondition`]: ome_ecs::reflect::FieldCondition
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
    type_id: TypeId,
    component: ComponentId,
    fields: &[(String, ReflectValue)],
    field_metas: Option<&'static [FieldMeta]>,
    euler_cache: &mut HashMap<EulerCacheKey, Vec3>,
    rotation_ctx: RotationContext,
    asset_catalog: &[AssetCatalogEntry],
    entities: &[EntityDisplayInfo],
) -> Vec<(String, ReflectValue)> {
    let mut edits = Vec::new();
    // Keyed on the component alone — see the note in `mod.rs`. The entity
    // used to be part of it, which renamed every widget in the grid the
    // moment the selection moved, while the grid stayed in the same place.
    egui::Grid::new(format!("fields_{component:?}"))
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            for (name, value) in fields {
                // A variant's own parameters only. Showing a capsule's
                // half_height while a sphere is selected implies it does
                // something; the value is still stored and still saved.
                if !field_is_shown(field_metas, name, fields) {
                    continue;
                }
                // Keyed on the field, not on its position in the grid.
                // `field_is_shown` hides a variant's unused parameters, so
                // the row count changes as a collider switches shape — and
                // with automatic ids that renames every widget below.
                //
                // One scope per cell, not one around the row: a scope
                // advances the grid's cursor, so wrapping both would put
                // the label and its editor in the same column.
                ui.push_id(("label", name), |ui| ui.label(name));
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
                            draw_quat_with_cache(ui, entity, type_id, name, *q, ctx, euler_cache)
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
    edits
}

/// Renders read-only display for component fields.
pub(super) fn draw_readonly_fields(
    ui: &mut egui::Ui,
    component: ComponentId,
    fields: &[(String, ReflectValue)],
    field_metas: Option<&'static [FieldMeta]>,
) {
    egui::Grid::new(format!("ro_fields_{component:?}"))
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            for (name, value) in fields {
                // A variant's own parameters only. Showing a capsule's
                // half_height while a sphere is selected implies it does
                // something; the value is still stored and still saved.
                if !field_is_shown(field_metas, name, fields) {
                    continue;
                }
                ui.push_id(("label", name), |ui| ui.label(name));
                ui.push_id(name, |ui| {
                    let choices = choices_for(field_metas, name);
                    let bits = bits_for(field_metas, name);
                    draw_readonly_value(ui, value, choices, bits);
                });
                ui.end_row();
            }
        });
}

#[cfg(test)]
mod condition_tests {
    use ome_ecs::reflect::{Reflect, ReflectValue};
    use ome_physics::components::{Collider, SHAPE_CAPSULE, SHAPE_CUBOID, SHAPE_SPHERE};

    use super::field_is_shown;

    /// `(name, current value)` for every field, the way the Inspector
    /// receives them.
    fn field_values(collider: &Collider) -> Vec<(String, ReflectValue)> {
        collider
            .reflect_fields()
            .iter()
            .filter_map(|meta| {
                collider
                    .reflect_get(meta.name)
                    .map(|value| (meta.name.to_owned(), value))
            })
            .collect()
    }

    /// The field names the Inspector would actually render for a shape.
    fn shown_fields(shape: u32) -> Vec<String> {
        let collider = Collider {
            shape,
            ..Default::default()
        };
        let fields = field_values(&collider);
        let metas = Some(collider.reflect_fields());
        fields
            .iter()
            .filter(|(name, _)| field_is_shown(metas, name, &fields))
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// What was reported: a sphere showed `half_extents` and `half_height`,
    /// which it ignores entirely, and they read as if they did something.
    #[test]
    fn a_sphere_shows_only_the_parameters_it_reads() {
        let shown = shown_fields(SHAPE_SPHERE);
        assert!(shown.contains(&"radius".to_owned()), "{shown:?}");
        assert!(shown.contains(&"center".to_owned()), "{shown:?}");
        assert!(
            !shown.contains(&"half_extents".to_owned()),
            "a sphere still offers half_extents: {shown:?}"
        );
        assert!(
            !shown.contains(&"half_height".to_owned()),
            "a sphere still offers half_height: {shown:?}"
        );
    }

    #[test]
    fn a_cuboid_shows_its_extents_and_not_the_round_parameters() {
        let shown = shown_fields(SHAPE_CUBOID);
        assert!(shown.contains(&"half_extents".to_owned()), "{shown:?}");
        assert!(!shown.contains(&"radius".to_owned()), "{shown:?}");
        assert!(!shown.contains(&"half_height".to_owned()), "{shown:?}");
    }

    #[test]
    fn a_capsule_shows_both_of_its_dimensions() {
        let shown = shown_fields(SHAPE_CAPSULE);
        assert!(shown.contains(&"radius".to_owned()), "{shown:?}");
        assert!(shown.contains(&"half_height".to_owned()), "{shown:?}");
        assert!(!shown.contains(&"half_extents".to_owned()), "{shown:?}");
    }

    /// `shape` and `center` apply to every variant, so they are never
    /// filtered — a condition is opt-in per field.
    #[test]
    fn the_shape_selector_and_centre_always_show() {
        for shape in [SHAPE_SPHERE, SHAPE_CUBOID, SHAPE_CAPSULE, 99] {
            let shown = shown_fields(shape);
            assert!(shown.contains(&"shape".to_owned()), "shape {shape}");
            assert!(shown.contains(&"center".to_owned()), "shape {shape}");
        }
    }

    /// Hiding is display only. Every field is still reflected, so it is
    /// still stored, still serialised, and still survives a scene
    /// round-trip — the reason the storage keeps all variants side by side
    /// in the first place.
    #[test]
    fn hidden_fields_are_still_stored_and_reflected() {
        let collider = Collider {
            shape: SHAPE_SPHERE,
            half_extents: glam::Vec3::splat(7.0),
            half_height: 3.0,
            ..Default::default()
        };
        let fields = field_values(&collider);

        // Present in reflection even though the Inspector hides them.
        let extents = fields.iter().find(|(n, _)| n == "half_extents");
        assert_eq!(
            extents.map(|(_, v)| v.clone()),
            Some(ReflectValue::Vec3(glam::Vec3::splat(7.0))),
            "a hidden field stopped being reflected"
        );
        assert!(
            !field_is_shown(Some(collider.reflect_fields()), "half_extents", &fields),
            "test is not exercising a hidden field"
        );
    }
}

#[cfg(test)]
mod name_editor_tests {
    use super::text_to_show;

    /// The reported bug, stated as the rule that caused it: with focus,
    /// the lagging snapshot must not win. It is one character behind what
    /// was just typed, and egui pulls the caret back to fit.
    #[test]
    fn the_typed_text_wins_while_the_field_has_focus() {
        let shown = text_to_show(true, Some("Doo".to_owned()), "Do");
        assert_eq!(shown, "Doo", "the stale snapshot overwrote the keystroke");
    }

    /// Focused with nothing typed yet — the first frame after clicking in.
    #[test]
    fn focus_without_a_buffer_falls_back_to_the_world() {
        assert_eq!(text_to_show(true, None, "Door frame"), "Door frame");
    }

    /// Unfocused, the world is authoritative: a rename the project
    /// altered or refused has to show what actually landed.
    #[test]
    fn the_world_wins_once_focus_is_gone() {
        let shown = text_to_show(false, Some("what I typed".to_owned()), "what landed");
        assert_eq!(shown, "what landed");
    }
}
