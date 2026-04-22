//! Per-`ReflectValue` editor widgets and choice helpers.
//!
//! `draw_value_widget` is the giant `match` over `ReflectValue` variants;
//! `draw_choice_dropdown` covers integer fields with `FieldChoice` hints;
//! `draw_readonly_value` is the non-interactive counterpart shared by
//! both single- and multi-entity rendering paths.

use ome_ecs::reflect::{FieldChoice, FieldMeta, ReflectValue};

/// Looks up the `choices` slice for a field by name. Returns an empty
/// slice if the metadata is missing or the field has no `choices` hint.
pub(super) fn choices_for(
    field_metas: Option<&'static [FieldMeta]>,
    name: &str,
) -> &'static [FieldChoice] {
    field_metas
        .and_then(|metas| metas.iter().find(|m| m.name == name))
        .map(|m| m.choices)
        .unwrap_or(&[])
}

/// Returns `(label, extensions)` when a String field's name suggests it
/// holds an asset path. Heuristic-based — same pattern as the `color`
/// detection on `Vec3`. Bridge until the asset handle system (#184)
/// makes this explicit via typed handles.
///
/// Format choices favour open standards with no licensing friction and
/// formats the engine actually plans to support:
/// - **Mesh**: glTF 2.0 only. Khronos open spec, supports animation,
///   skeletons, materials, scenes — drops the legacy `.obj`.
/// - **Texture**: LDR (PNG, JPEG), GPU compressed (KTX2), HDR (EXR,
///   Radiance HDR). Drops `.tga` — legacy with no real advantage.
/// - **Audio**: Xiph (Vorbis, FLAC) plus PCM `.wav`. Drops `.mp3`
///   because `kira` gates it behind a feature flag and the historical
///   patent baggage adds zero value when Vorbis covers the same niche.
/// - **Material**: RON only. Same format as `.ome_scene`, handles
///   nested structures cleanly. Drops TOML.
///
/// `extensions` is empty when the kind is recognised but no specific
/// filter applies (generic `*_path` / `*_file` fields). The dialog will
/// still show the file picker, just without a type filter.
fn asset_filter_for(field_name: &str) -> Option<(&'static str, &'static [&'static str])> {
    let n = field_name.to_lowercase();
    if n.contains("mesh") {
        Some(("Mesh", &["gltf", "glb"]))
    } else if n.contains("texture") || n.contains("image") {
        Some(("Texture", &["png", "jpg", "jpeg", "ktx2", "exr", "hdr"]))
    } else if n.contains("audio") || n.contains("sound") {
        Some(("Audio", &["ogg", "wav", "flac"]))
    } else if n.contains("scene") {
        Some(("Scene", &["ome_scene"]))
    } else if n.contains("shader") {
        Some(("Shader", &["wgsl"]))
    } else if n.contains("material") {
        Some(("Material", &["ron"]))
    } else if n.ends_with("_path") || n.ends_with("_file") {
        Some(("File", &[]))
    } else {
        None
    }
}

/// Renders a read-only display for a single value. If the field has a
/// `choices` hint, prefer the matching label over the raw numeric value.
pub(super) fn draw_readonly_value(
    ui: &mut egui::Ui,
    value: &ReflectValue,
    choices: &'static [FieldChoice],
) {
    if let Some(label) = choice_label_for(value, choices) {
        ui.weak(label);
    } else {
        ui.weak(format!("{value}"));
    }
}

/// Returns the `choices` label for an integer-valued field, if any.
fn choice_label_for(
    value: &ReflectValue,
    choices: &'static [FieldChoice],
) -> Option<&'static str> {
    let current = reflect_value_as_i64(value)?;
    choices
        .iter()
        .find(|c| c.value == current)
        .map(|c| c.label)
}

/// Converts an integer [`ReflectValue`] into `i64` for dropdown matching.
fn reflect_value_as_i64(value: &ReflectValue) -> Option<i64> {
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

/// Reconstructs an integer [`ReflectValue`] of the same kind as `template`
/// from an `i64` value produced by the dropdown. Returns `None` for
/// non-integer templates.
fn reflect_value_from_i64(template: &ReflectValue, v: i64) -> Option<ReflectValue> {
    match template {
        ReflectValue::U8(_) => Some(ReflectValue::U8(v.clamp(0, u8::MAX as i64) as u8)),
        ReflectValue::U16(_) => Some(ReflectValue::U16(v.clamp(0, u16::MAX as i64) as u16)),
        ReflectValue::U32(_) => Some(ReflectValue::U32(v.clamp(0, u32::MAX as i64) as u32)),
        ReflectValue::U64(_) => Some(ReflectValue::U64(v.max(0) as u64)),
        ReflectValue::I8(_) => {
            Some(ReflectValue::I8(v.clamp(i8::MIN as i64, i8::MAX as i64) as i8))
        }
        ReflectValue::I16(_) => Some(ReflectValue::I16(
            v.clamp(i16::MIN as i64, i16::MAX as i64) as i16,
        )),
        ReflectValue::I32(_) => Some(ReflectValue::I32(
            v.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
        )),
        ReflectValue::I64(_) => Some(ReflectValue::I64(v)),
        _ => None,
    }
}

/// Renders a dropdown for an integer field with `choices` metadata.
/// Returns `Some(new_value)` when the user picks a different entry.
fn draw_choice_dropdown(
    ui: &mut egui::Ui,
    value: &ReflectValue,
    choices: &'static [FieldChoice],
    field_name: &str,
) -> Option<ReflectValue> {
    let current = reflect_value_as_i64(value)?;
    let selected_label = choices
        .iter()
        .find(|c| c.value == current)
        .map(|c| c.label)
        .unwrap_or("(unknown)");
    let mut picked: Option<i64> = None;
    egui::ComboBox::from_id_salt(("choice_dropdown", field_name))
        .selected_text(selected_label)
        .show_ui(ui, |ui| {
            for choice in choices {
                if ui
                    .selectable_label(choice.value == current, choice.label)
                    .clicked()
                {
                    picked = Some(choice.value);
                }
            }
        });
    let new_val = picked?;
    if new_val == current {
        return None;
    }
    reflect_value_from_i64(value, new_val)
}

/// Draws an editable widget for a single reflected value.
/// Returns `Some(new_value)` if the user modified it.
/// `field_name` is used to detect color fields and show a color picker.
/// `choices` renders integer fields as a dropdown when non-empty.
pub(super) fn draw_value_widget(
    ui: &mut egui::Ui,
    value: &ReflectValue,
    field_name: &str,
    choices: &'static [FieldChoice],
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
                        if !val.is_empty() {
                            if let Some(parent) = std::path::Path::new(&val).parent() {
                                if parent.exists() {
                                    dialog = dialog.set_directory(parent);
                                }
                            }
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

#[cfg(test)]
mod tests {
    use super::asset_filter_for;

    #[test]
    fn detects_mesh_field() {
        let (label, exts) = asset_filter_for("mesh").expect("mesh recognised");
        assert_eq!(label, "Mesh");
        assert!(exts.contains(&"gltf"));
        assert!(exts.contains(&"glb"));
        // Legacy formats deliberately excluded — see asset_filter_for docstring.
        assert!(!exts.contains(&"obj"));
    }

    #[test]
    fn excludes_legacy_and_licensed_audio() {
        let (_, exts) = asset_filter_for("audio").expect("audio recognised");
        assert!(exts.contains(&"ogg"));
        assert!(exts.contains(&"flac"));
        assert!(!exts.contains(&"mp3"));
    }

    #[test]
    fn material_uses_ron_only() {
        let (_, exts) = asset_filter_for("material").expect("material recognised");
        assert_eq!(exts, &["ron"]);
    }

    #[test]
    fn detects_compound_field_names() {
        // Real components use suffixed names like `mesh_path`, `texture_handle`, etc.
        assert!(asset_filter_for("mesh_path").is_some());
        assert!(asset_filter_for("diffuse_texture").is_some());
        assert!(asset_filter_for("background_audio").is_some());
        assert!(asset_filter_for("startup_scene").is_some());
        assert!(asset_filter_for("vertex_shader").is_some());
        assert!(asset_filter_for("base_material").is_some());
    }

    #[test]
    fn case_insensitive() {
        assert!(asset_filter_for("MeshPath").is_some());
        assert!(asset_filter_for("DIFFUSE_TEXTURE").is_some());
    }

    #[test]
    fn generic_path_suffix_falls_back_to_no_filter() {
        let (label, exts) = asset_filter_for("config_path").expect("recognised");
        assert_eq!(label, "File");
        assert!(exts.is_empty());
    }

    #[test]
    fn non_asset_strings_return_none() {
        // Plain string fields like Name.value must NOT trigger a picker.
        assert!(asset_filter_for("value").is_none());
        assert!(asset_filter_for("name").is_none());
        assert!(asset_filter_for("description").is_none());
    }
}
