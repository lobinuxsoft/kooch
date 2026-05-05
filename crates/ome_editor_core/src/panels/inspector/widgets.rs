//! Per-`ReflectValue` editor widgets and choice helpers.
//!
//! [`draw_value_widget`] is the giant `match` over `ReflectValue`
//! variants; [`choices::draw_choice_dropdown`] covers integer fields with
//! `FieldChoice` hints (used internally); [`draw_readonly_value`] is the
//! non-interactive counterpart shared by both single- and multi-entity
//! rendering paths.

mod asset;
mod choices;

use ome_core::Guid;
use ome_ecs::reflect::{FieldChoice, ReflectValue};

pub(crate) use self::asset::{AssetCatalogEntry, AssetSource};
pub(super) use self::choices::{choices_for, draw_readonly_value};

use self::asset::asset_filter_for;
use self::choices::draw_choice_dropdown;

/// Renders the typed asset-reference picker for a `ReflectValue::AssetRef`
/// field. Returns `Some(new_value)` when the user picks a different
/// asset (or clears the field), otherwise `None`.
///
/// Layout follows the Unity inspector's object-field convention:
/// - Selected state shows the current asset's basename + source tag,
///   or `(None)` / `(missing: <guid>)` when unresolvable.
/// - Dropdown opens a search field at the top, then the filtered
///   list. Each row shows `display_name [source]` with the full
///   path on hover.
fn draw_asset_picker(
    ui: &mut egui::Ui,
    current: Option<Guid>,
    asset_type: &str,
    catalog: &[AssetCatalogEntry],
) -> Option<ReflectValue> {
    let filtered: Vec<&AssetCatalogEntry> = catalog
        .iter()
        .filter(|e| e.type_name == asset_type)
        .collect();

    let current_entry = current.and_then(|g| filtered.iter().find(|e| e.guid == g).copied());

    let selected_text = match (current, current_entry) {
        (Some(_), Some(entry)) => format!("{} [{}]", entry.display_name, entry.source.label()),
        (Some(g), None) => format!("(missing: {g})"),
        (None, _) => "(None)".to_owned(),
    };

    let mut new_value: Option<ReflectValue> = None;
    let search_id = ui.id().with(("asset_picker_search", asset_type));

    let combo_response = egui::ComboBox::from_id_salt(("asset_picker", asset_type))
        .selected_text(selected_text)
        .show_ui(ui, |ui| {
            // Search box — Unity-style. Persisted in egui memory so
            // the query survives close-and-reopen of the same
            // dropdown across frames.
            let mut query: String = ui
                .ctx()
                .data(|d| d.get_temp::<String>(search_id))
                .unwrap_or_default();
            let search_resp = ui.add(
                egui::TextEdit::singleline(&mut query)
                    .desired_width(f32::INFINITY)
                    .hint_text("\u{1f50d} Search…"),
            );
            if search_resp.changed() {
                ui.ctx().data_mut(|d| d.insert_temp(search_id, query.clone()));
            }

            ui.separator();

            // The "(None)" entry — clears the assignment. Always
            // visible, never filtered out.
            if ui.selectable_label(current.is_none(), "(None)").clicked()
                && current.is_some()
            {
                new_value = Some(ReflectValue::AssetRef {
                    guid: None,
                    asset_type: asset_type.to_owned(),
                });
            }

            if filtered.is_empty() {
                ui.weak(format!("(no {asset_type} assets registered)"));
                return;
            }

            let needle = query.trim().to_lowercase();
            let matches_query = |entry: &AssetCatalogEntry| -> bool {
                if needle.is_empty() {
                    return true;
                }
                entry.display_name.to_lowercase().contains(&needle)
                    || entry.path.display().to_string().to_lowercase().contains(&needle)
            };

            let mut shown = 0usize;
            for entry in &filtered {
                if !matches_query(entry) {
                    continue;
                }
                let selected = current == Some(entry.guid);
                let label = format!(
                    "{}  [{}]",
                    entry.display_name,
                    entry.source.label(),
                );
                let resp = ui
                    .selectable_label(selected, label)
                    .on_hover_text(entry.path.display().to_string());
                if !selected && resp.clicked() {
                    new_value = Some(ReflectValue::AssetRef {
                        guid: Some(entry.guid),
                        asset_type: asset_type.to_owned(),
                    });
                }
                shown += 1;
            }
            if shown == 0 {
                ui.weak("(no match)");
            }
        });
    let _ = combo_response;

    new_value
}

/// Draws an editable widget for a single reflected value.
/// Returns `Some(new_value)` if the user modified it.
/// `field_name` is used to detect color fields and show a color picker.
/// `choices` renders integer fields as a dropdown when non-empty.
/// `asset_catalog` is the per-frame snapshot of `AssetDatabase` the
/// `AssetRef` widget filters when populating its picker dropdown.
pub(super) fn draw_value_widget(
    ui: &mut egui::Ui,
    value: &ReflectValue,
    field_name: &str,
    choices: &'static [FieldChoice],
    asset_catalog: &[AssetCatalogEntry],
) -> Option<ReflectValue> {
    if !choices.is_empty()
        && let Some(new_value) = draw_choice_dropdown(ui, value, choices, field_name)
    {
        return Some(new_value);
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
            let resp = ui.add(
                egui::DragValue::new(&mut val).range(i8::MIN as i64..=i8::MAX as i64),
            );
            resp.changed().then_some(ReflectValue::I8(val as i8))
        }
        ReflectValue::I16(v) => {
            let mut val = *v as i64;
            let resp = ui.add(
                egui::DragValue::new(&mut val).range(i16::MIN as i64..=i16::MAX as i64),
            );
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
                        let mut dialog =
                            rfd::FileDialog::new().set_title(format!("Pick {label}"));
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
                        ui.colored_label(
                            egui::Color32::from_rgb(240, 180, 40),
                            "\u{26a0}",
                        )
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
