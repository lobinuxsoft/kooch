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
    AssetCatalogEntry, bits_for, choices_for, draw_readonly_value, draw_value_widget,
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

    let mut val = current.clone();
    ui.horizontal(|ui| {
        ui.label("Name");
        if ui.text_edit_singleline(&mut val).changed() {
            actions.push(EditorAction::SetField {
                entity,
                component: comp.component,
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

pub(super) fn draw_reflected_fields(
    ui: &mut egui::Ui,
    entity: Entity,
    type_id: TypeId,
    component: ComponentId,
    fields: &[(String, ReflectValue)],
    field_metas: Option<&'static [FieldMeta]>,
    euler_cache: &mut HashMap<EulerCacheKey, Vec3>,
    rotation_ctx: RotationContext,
    actions: &mut Vec<EditorAction>,
    asset_catalog: &[AssetCatalogEntry],
) {
    egui::Grid::new(format!("fields_{:?}_{}", component, entity.index()))
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
                ui.label(name);
                let choices = choices_for(field_metas, name);
                let bits = bits_for(field_metas, name);
                let new_value = match value {
                    ReflectValue::Quat(q) => {
                        let ctx = if is_transform_rotation(type_id, name) {
                            rotation_ctx
                        } else {
                            RotationContext::local_only()
                        };
                        draw_quat_with_cache(ui, entity, type_id, name, *q, ctx, euler_cache)
                    }
                    _ => draw_value_widget(ui, value, name, choices, bits, asset_catalog),
                };
                if let Some(new_value) = new_value {
                    actions.push(EditorAction::SetField {
                        entity,
                        component,
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
    component: ComponentId,
    fields: &[(String, ReflectValue)],
    field_metas: Option<&'static [FieldMeta]>,
) {
    egui::Grid::new(format!("ro_fields_{:?}_{}", component, entity.index()))
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
                ui.label(name);
                let choices = choices_for(field_metas, name);
                let bits = bits_for(field_metas, name);
                draw_readonly_value(ui, value, choices);
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
