//! Editable widget dispatcher: the giant match over `ReflectValue`.

use ome_ecs::reflect::{FieldChoice, ReflectValue};

use super::asset::{AssetCatalogEntry, asset_filter_for};
use super::asset_picker::draw_asset_picker;
use super::choices::{draw_bitmask, draw_choice_dropdown};
use super::entity_picker::draw_entity_picker;
use crate::state::EntityDisplayInfo;

/// Everything a widget needs about the field it is drawing, other than
/// the value itself.
///
/// A struct rather than a parameter list: the reference picker needs the
/// scene's entities and the field's `requires` hint, and eight positional
/// arguments threaded through three call sites is how the next one gets
/// passed in the wrong order.
pub(in crate::panels::inspector) struct FieldContext<'a> {
    /// Field name. Also the colour-field and asset-path heuristic's input.
    pub name: &'a str,
    /// Renders an integer field as a dropdown when non-empty.
    pub choices: &'static [FieldChoice],
    /// Renders an integer field as named checkboxes when non-empty. A
    /// field is one of a set or a combination of them, never both, so
    /// `choices` wins if somehow given both.
    pub bits: &'static [FieldChoice],
    /// Per-frame snapshot of `AssetDatabase`, filtered by the `AssetRef`
    /// widget when it populates its dropdown.
    pub assets: &'a [AssetCatalogEntry],
    /// The entities the reference picker offers.
    pub entities: &'a [EntityDisplayInfo],
    /// `FieldMeta::requires`: a component the reference's target must
    /// carry, or `""` for no constraint.
    pub requires: &'a str,
}

/// Draws an editable widget for a single reflected value.
/// Returns `Some(new_value)` if the user modified it.
pub(in crate::panels::inspector) fn draw_value_widget(
    ui: &mut egui::Ui,
    value: &ReflectValue,
    field: &FieldContext<'_>,
) -> Option<ReflectValue> {
    let FieldContext {
        name: field_name,
        choices,
        bits,
        assets: asset_catalog,
        ..
    } = *field;
    if !choices.is_empty() {
        // Returns `None` while the popup is merely open, so the dropdown
        // cannot fall through to the numeric widget behind it.
        return draw_choice_dropdown(ui, value, choices, field_name);
    }
    if !bits.is_empty() {
        return draw_bitmask(ui, value, bits, field_name);
    }
    match value {
        ReflectValue::F32(v) => {
            let mut val = *v;
            let resp = ui.add(egui::DragValue::new(&mut val).speed(0.1));
            resp.changed().then_some(ReflectValue::F32(val))
        }
        ReflectValue::F64(v) => {
            let mut val = *v as f32;
            let resp = ui.add(egui::DragValue::new(&mut val).speed(0.1));
            resp.changed().then_some(ReflectValue::F64(val as f64))
        }
        ReflectValue::U8(v) => {
            let mut val = *v as i64;
            let resp = ui.add(egui::DragValue::new(&mut val).range(0..=u8::MAX as i64));
            resp.changed().then_some(ReflectValue::U8(val as u8))
        }
        ReflectValue::U16(v) => {
            let mut val = *v as i64;
            let resp = ui.add(egui::DragValue::new(&mut val).range(0..=u16::MAX as i64));
            resp.changed().then_some(ReflectValue::U16(val as u16))
        }
        ReflectValue::U32(v) => {
            let mut val = *v as i64;
            let resp = ui.add(egui::DragValue::new(&mut val).range(0..=u32::MAX as i64));
            resp.changed().then_some(ReflectValue::U32(val as u32))
        }
        ReflectValue::U64(v) => {
            let mut val = *v as i64;
            let resp = ui.add(egui::DragValue::new(&mut val));
            resp.changed()
                .then_some(ReflectValue::U64(val.max(0) as u64))
        }
        ReflectValue::I8(v) => {
            let mut val = *v as i64;
            let resp =
                ui.add(egui::DragValue::new(&mut val).range(i8::MIN as i64..=i8::MAX as i64));
            resp.changed().then_some(ReflectValue::I8(val as i8))
        }
        ReflectValue::I16(v) => {
            let mut val = *v as i64;
            let resp =
                ui.add(egui::DragValue::new(&mut val).range(i16::MIN as i64..=i16::MAX as i64));
            resp.changed().then_some(ReflectValue::I16(val as i16))
        }
        ReflectValue::I32(v) => {
            let mut val = *v;
            let resp = ui.add(egui::DragValue::new(&mut val));
            resp.changed().then_some(ReflectValue::I32(val))
        }
        ReflectValue::I64(v) => {
            let mut val = *v;
            let resp = ui.add(egui::DragValue::new(&mut val));
            resp.changed().then_some(ReflectValue::I64(val))
        }
        ReflectValue::Bool(v) => {
            let mut val = *v;
            let resp = ui.checkbox(&mut val, "");
            resp.changed().then_some(ReflectValue::Bool(val))
        }
        ReflectValue::String(v) => {
            let mut val = v.clone();
            let mut changed = false;
            if let Some((label, exts)) = asset_filter_for(field_name) {
                // Asset path field: file picker is the ONLY way to set
                // the value (no manual editing — too error-prone). The
                // path itself is shown as a truncating label with the
                // full path on hover.
                ui.horizontal(|ui| {
                    if ui
                        .button("\u{1f4c1}")
                        .on_hover_text(format!("Pick {label} file"))
                        .clicked()
                    {
                        let mut dialog = rfd::FileDialog::new().set_title(format!("Pick {label}"));
                        if !exts.is_empty() {
                            // Include the extension list in the filter label
                            // so the native dialog's "file type" dropdown is
                            // self-documenting — the GTK/KDE dropdown only
                            // shows the label, not the underlying ext list.
                            let exts_display = exts
                                .iter()
                                .map(|e| format!("*.{e}"))
                                .collect::<Vec<_>>()
                                .join(", ");
                            let label_with_exts = format!("{label} ({exts_display})");
                            dialog = dialog.add_filter(label_with_exts, exts);
                        }
                        if !val.is_empty()
                            && let Some(parent) = std::path::Path::new(&val).parent()
                            && parent.exists()
                        {
                            dialog = dialog.set_directory(parent);
                        }
                        if let Some(picked) = dialog.pick_file() {
                            val = picked.to_string_lossy().into_owned();
                            changed = true;
                        }
                    }
                    if !val.is_empty()
                        && ui
                            .small_button("\u{2715}")
                            .on_hover_text("Clear path")
                            .clicked()
                    {
                        val.clear();
                        changed = true;
                    }
                    if val.is_empty() {
                        ui.weak("(empty)");
                    } else {
                        ui.add(egui::Label::new(&val).truncate())
                            .on_hover_text(&val);
                    }
                });
            } else {
                changed |= ui.text_edit_singleline(&mut val).changed();
            }
            changed.then_some(ReflectValue::String(val))
        }
        ReflectValue::Vec2(v) => {
            let mut x = v.x;
            let mut y = v.y;
            let mut changed = false;
            ui.horizontal(|ui| {
                ui.label("x");
                changed |= ui.add(egui::DragValue::new(&mut x).speed(0.1)).changed();
                ui.label("y");
                changed |= ui.add(egui::DragValue::new(&mut y).speed(0.1)).changed();
            });
            changed.then_some(ReflectValue::Vec2(glam::Vec2::new(x, y)))
        }
        ReflectValue::Vec3(v) => {
            if field_name.contains("color") {
                let mut rgb = [v.x, v.y, v.z];
                let resp = ui.color_edit_button_rgb(&mut rgb);
                resp.changed()
                    .then_some(ReflectValue::Vec3(glam::Vec3::new(rgb[0], rgb[1], rgb[2])))
            } else {
                let mut x = v.x;
                let mut y = v.y;
                let mut z = v.z;
                let mut changed = false;
                ui.horizontal(|ui| {
                    ui.label("x");
                    changed |= ui.add(egui::DragValue::new(&mut x).speed(0.1)).changed();
                    ui.label("y");
                    changed |= ui.add(egui::DragValue::new(&mut y).speed(0.1)).changed();
                    ui.label("z");
                    changed |= ui.add(egui::DragValue::new(&mut z).speed(0.1)).changed();
                });
                changed.then_some(ReflectValue::Vec3(glam::Vec3::new(x, y, z)))
            }
        }
        ReflectValue::Vec4(v) => {
            let is_color = field_name.contains("color");
            if is_color {
                let mut rgba = [v.x, v.y, v.z, v.w];
                let resp = ui.color_edit_button_rgba_unmultiplied(&mut rgba);
                resp.changed().then_some(ReflectValue::Vec4(glam::Vec4::new(
                    rgba[0], rgba[1], rgba[2], rgba[3],
                )))
            } else {
                let mut vals = [v.x, v.y, v.z, v.w];
                let labels = ["x", "y", "z", "w"];
                let mut changed = false;
                ui.horizontal(|ui| {
                    for (i, label) in labels.iter().enumerate() {
                        ui.label(*label);
                        changed |= ui
                            .add(egui::DragValue::new(&mut vals[i]).speed(0.1))
                            .changed();
                    }
                });
                changed.then_some(ReflectValue::Vec4(glam::Vec4::new(
                    vals[0], vals[1], vals[2], vals[3],
                )))
            }
        }
        ReflectValue::Quat(v) => {
            // Display as Euler angles (degrees) for intuitive editing.
            let (rx, ry, rz) = v.to_euler(glam::EulerRot::XYZ);
            let mut dx = rx.to_degrees() + 0.0; // eliminate -0.0
            let mut dy = ry.to_degrees() + 0.0;
            let mut dz = rz.to_degrees() + 0.0;
            let mut changed = false;
            ui.horizontal(|ui| {
                ui.label("x");
                changed |= ui
                    .add(egui::DragValue::new(&mut dx).speed(0.5).suffix("\u{00b0}"))
                    .changed();
                ui.label("y");
                changed |= ui
                    .add(egui::DragValue::new(&mut dy).speed(0.5).suffix("\u{00b0}"))
                    .changed();
                ui.label("z");
                changed |= ui
                    .add(egui::DragValue::new(&mut dz).speed(0.5).suffix("\u{00b0}"))
                    .changed();
            });
            changed.then_some(ReflectValue::Quat(glam::Quat::from_euler(
                glam::EulerRot::XYZ,
                dx.to_radians(),
                dy.to_radians(),
                dz.to_radians(),
            )))
        }
        ReflectValue::AssetRef { guid, asset_type } => {
            draw_asset_picker(ui, *guid, asset_type, asset_catalog)
        }
        ReflectValue::EntityRef(reference) => {
            draw_entity_picker(ui, *reference, field.entities, field.requires, field_name)
        }
        ReflectValue::Mat4(m) => {
            let (scale, rotation, translation) = m.to_scale_rotation_translation();
            let (ex, ey, ez) = rotation.to_euler(glam::EulerRot::XYZ);
            // Reuse GlobalTransform's shear detector so the inspector
            // flags the same configurations the engine considers risky
            // for TRS round-trip. See #214 for the full discussion.
            let gt = ome_ecs::hierarchy::GlobalTransform { matrix: *m };
            let shear = gt.has_shear(1e-4);
            ui.vertical(|ui| {
                ui.label(format!(
                    "translation ({:.3}, {:.3}, {:.3})",
                    translation.x, translation.y, translation.z
                ));
                ui.label(format!(
                    "rotation    ({:.2}\u{00b0}, {:.2}\u{00b0}, {:.2}\u{00b0})",
                    ex.to_degrees(),
                    ey.to_degrees(),
                    ez.to_degrees()
                ));
                ui.horizontal(|ui| {
                    ui.label(format!(
                        "lossy_scale ({:.3}, {:.3}, {:.3})",
                        scale.x, scale.y, scale.z
                    ));
                    if shear {
                        ui.colored_label(egui::Color32::from_rgb(240, 180, 40), "\u{26a0}")
                            .on_hover_text(
                                "Shear detected in this matrix. Non-uniform \
                             parent scale composed with a rotated child \
                             produces shear that `Transform { scale }` \
                             cannot represent. The values above are a \
                             best-fit decomposition and will NOT round-trip \
                             through TRS. See issue #214.",
                            );
                    }
                });
            });
            None
        }
    }
}
