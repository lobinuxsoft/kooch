//! The material editor: PBR fields, a shader's declared parameters and their textures.

use super::*;

/// Editable material parameters. Emits a single `EditMaterial` action
/// carrying the full edited material whenever any widget changes this
/// frame. Persisting / live GPU sync is the action handler's job.
pub(super) fn draw_material_editor(
    ui: &mut egui::Ui,
    guid: Guid,
    mat: &Material,
    shader: &MaterialShader,
    catalog: &[AssetCatalogEntry],
    actions: &mut Vec<EditorAction>,
) {
    let mut edited = mat.clone();
    let mut changed = false;
    // Set on the frame a drag ends. That frame usually reports no change
    // — the value stopped moving before the button came up — so it is a
    // second signal rather than a kind of `changed`.
    let mut released = false;

    ui.horizontal(|ui| {
        ui.label("Shader").on_hover_text(
            "The .shader this material shades with. (None) is the engine's PBR surface",
        );
        let picked = ui
            .push_id("shader", |ui| {
                draw_asset_picker(ui, edited.shader, SHADER_TYPE_NAME, catalog)
            })
            .inner;
        if let Some(ReflectValue::AssetRef { guid, .. }) = picked {
            edited.shader = guid;
            changed = true;
        }
    });
    ui.separator();

    match shader {
        MaterialShader::Default => {
            let (pbr_changed, pbr_released) = draw_pbr_fields(ui, guid, &mut edited, catalog);
            changed |= pbr_changed;
            released |= pbr_released;
        }
        MaterialShader::Declares(params) => {
            let (param_changed, param_released) =
                draw_shader_params(ui, guid, &mut edited, params, catalog);
            changed |= param_changed;
            released |= param_released;
        }
        MaterialShader::Unavailable => {
            ui.weak("The shader is missing or does not parse — the Console says why.");
        }
    }

    if !changed && !released {
        return;
    }
    // A change with nothing held down is already final: typing a number,
    // picking a texture, clicking a swatch. Only a drag needs the wait,
    // and `released` is what ends it.
    let held = ui.input(|input| input.pointer.any_down());
    actions.push(EditorAction::EditMaterial {
        guid,
        material: edited,
        commit: released || !held,
    });
}

/// The engine surface's built-in fields. Returns (changed, drag released).
pub(super) fn draw_pbr_fields(
    ui: &mut egui::Ui,
    guid: Guid,
    edited: &mut Material,
    catalog: &[AssetCatalogEntry],
) -> (bool, bool) {
    let mut changed = false;
    let mut released = false;
    egui::Grid::new(("material_editor", guid))
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label("Base color");
            let response = ui.color_edit_button_rgba_unmultiplied(&mut edited.base_color);
            changed |= response.changed();
            released |= response.drag_stopped();
            ui.end_row();

            ui.label("Metallic");
            let response = ui.add(egui::Slider::new(&mut edited.metallic, 0.0..=1.0));
            changed |= response.changed();
            released |= response.drag_stopped();
            ui.end_row();

            ui.label("Roughness");
            let response = ui.add(egui::Slider::new(&mut edited.roughness, 0.0..=1.0));
            changed |= response.changed();
            released |= response.drag_stopped();
            ui.end_row();

            ui.label("Emissive");
            let response = ui.add(
                crate::numeric::drag(&mut edited.emissive)
                    .speed(0.01)
                    .range(0.0..=100.0),
            );
            changed |= response.changed();
            released |= response.drag_stopped();
            ui.end_row();
        });

    ui.separator();
    ui.label("Textures");
    egui::Grid::new(("material_uv", guid))
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            // Above the slots on purpose: it applies to all three, and
            // reading it after them invites the idea that it belongs to
            // the last one.
            ui.label("Tiling");
            ui.horizontal(|ui| {
                for axis in 0..2 {
                    let response = ui.add(
                        crate::numeric::drag(&mut edited.uv_scale[axis])
                            .speed(0.05)
                            .range(0.001..=1024.0),
                    );
                    changed |= response.changed();
                    released |= response.drag_stopped();
                }
            });
            ui.end_row();

            ui.label("Offset");
            ui.horizontal(|ui| {
                for axis in 0..2 {
                    let response =
                        ui.add(crate::numeric::drag(&mut edited.uv_offset[axis]).speed(0.01));
                    changed |= response.changed();
                    released |= response.drag_stopped();
                }
            });
            ui.end_row();
        });
    changed |= texture_row(ui, "Albedo", &mut edited.albedo, catalog);
    changed |= texture_row(ui, "Normal", &mut edited.normal, catalog);
    changed |= texture_row(ui, "Metal/Rough", &mut edited.metal_roughness, catalog);
    (changed, released)
}

/// A custom shader's declared parameters, each at its value or its default. Returns (changed, drag
/// released).
pub(super) fn draw_shader_params(
    ui: &mut egui::Ui,
    guid: Guid,
    edited: &mut Material,
    params: &[ShaderParam],
    catalog: &[AssetCatalogEntry],
) -> (bool, bool) {
    if params.is_empty() {
        ui.weak("This shader declares no parameters.");
        return (false, false);
    }
    let mut changed = false;
    let mut released = false;
    let mut track = |response: egui::Response| {
        changed |= response.changed();
        released |= response.drag_stopped();
    };
    let mut textures = Vec::new();
    egui::Grid::new(("material_params", guid))
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            for param in params {
                if param.kind == ParamKind::Texture {
                    textures.push(param);
                    continue;
                }
                let mut value = match edited.values.get(&param.name) {
                    Some(ParamValue::Number(n)) => *n,
                    _ => param.default,
                };
                ui.label(&param.name);
                match (param.kind, param.range) {
                    (ParamKind::Float, Some([lo, hi])) => {
                        track(ui.add(egui::Slider::new(&mut value[0], lo..=hi)));
                    }
                    (ParamKind::Int, Some([lo, hi])) => {
                        track(
                            ui.add(
                                egui::Slider::new(&mut value[0], lo..=hi)
                                    .step_by(1.0)
                                    .fixed_decimals(0),
                            ),
                        );
                    }
                    (ParamKind::Int, None) => {
                        track(
                            ui.add(
                                egui::DragValue::new(&mut value[0])
                                    .speed(1.0)
                                    .fixed_decimals(0),
                            ),
                        );
                    }
                    (ParamKind::Color, _) => {
                        track(ui.color_edit_button_rgba_unmultiplied(&mut value));
                    }
                    (kind, _) => {
                        ui.horizontal(|ui| {
                            for component in value.iter_mut().take(kind.width() as usize) {
                                track(ui.add(crate::numeric::drag(component).speed(0.01)));
                            }
                        });
                    }
                }
                ui.end_row();
                let stored = match edited.values.get(&param.name) {
                    Some(ParamValue::Number(n)) => *n,
                    _ => param.default,
                };
                if value != stored {
                    edited
                        .values
                        .insert(param.name.clone(), ParamValue::Number(value));
                }
            }
        });
    for param in textures {
        let mut texture = match edited.values.get(&param.name) {
            Some(ParamValue::Texture(guid)) => *guid,
            _ => None,
        };
        if texture_row(ui, &param.name, &mut texture, catalog) {
            edited
                .values
                .insert(param.name.clone(), ParamValue::Texture(texture));
            changed = true;
        }
    }
    (changed, released)
}

/// One texture slot row backed by the shared typed asset picker. Returns `true` when the assignment
/// changed this frame.
pub(super) fn texture_row(
    ui: &mut egui::Ui,
    label: &str,
    field: &mut Option<Guid>,
    catalog: &[AssetCatalogEntry],
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        let picked = ui
            .push_id(label, |ui| {
                draw_asset_picker(ui, *field, IMAGE_TYPE, catalog)
            })
            .inner;
        if let Some(ReflectValue::AssetRef { guid, .. }) = picked {
            *field = guid;
            changed = true;
        }
    });
    changed
}
