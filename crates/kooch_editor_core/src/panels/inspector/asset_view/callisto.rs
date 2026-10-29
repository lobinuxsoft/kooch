//! The material's light-model controls (#1657), in the three tiers the Callisto talk proposes
//! (slide 98): base for everyone, tints and specular for material artists, the rest for tuning.

use kooch_render::material::Callisto;

/// Returns (changed, drag released).
pub(super) fn draw_callisto(
    ui: &mut egui::Ui,
    guid: kooch_core::Guid,
    callisto: &mut Callisto,
) -> (bool, bool) {
    let mut changed = false;
    let mut released = false;
    let mut reset = false;
    let mut track = |response: egui::Response| {
        changed |= response.changed();
        released |= response.drag_stopped();
    };
    egui::CollapsingHeader::new("Light model")
        .id_salt(("callisto", guid))
        .show(ui, |ui| {
            ui.weak("Callisto BRDF. The defaults are plain Lambert + GGX.");
            egui::Grid::new(("callisto_base", guid))
                .num_columns(2)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    intensity(
                        ui,
                        "Diffuse Fresnel",
                        &mut callisto.fresnel,
                        0.0..=256.0,
                        &mut track,
                    )
                    .on_hover_text("Brightens the diffuse where light grazes: eyes, teeth");
                    unit(
                        ui,
                        "Fresnel falloff",
                        &mut callisto.fresnel_falloff,
                        &mut track,
                    );
                    intensity(
                        ui,
                        "Retroreflection",
                        &mut callisto.retro,
                        0.0..=256.0,
                        &mut track,
                    )
                    .on_hover_text("Brightens the diffuse lit from behind the viewer: skin, cloth");
                    unit(ui, "Retro falloff", &mut callisto.retro_falloff, &mut track);
                    intensity(
                        ui,
                        "Smooth terminator",
                        &mut callisto.terminator,
                        -1.0..=1.0,
                        &mut track,
                    )
                    .on_hover_text("Softens the edge between light and shadow");
                });

            egui::CollapsingHeader::new("Advanced")
                .id_salt(("callisto_advanced", guid))
                .show(ui, |ui| {
                    egui::Grid::new(("callisto_advanced_grid", guid))
                        .num_columns(2)
                        .spacing([8.0, 4.0])
                        .show(ui, |ui| {
                            tint(ui, "Fresnel tint", &mut callisto.fresnel_tint, &mut track);
                            tint(ui, "Retro tint", &mut callisto.retro_tint, &mut track);
                            tint(
                                ui,
                                "Terminator tint",
                                &mut callisto.terminator_tint,
                                &mut track,
                            );
                            unit(
                                ui,
                                "Specular falloff",
                                &mut callisto.specular_falloff,
                                &mut track,
                            );
                            unit(
                                ui,
                                "Fresnel tangent",
                                &mut callisto.fresnel_tangent,
                                &mut track,
                            );
                            unit(ui, "Retro tangent", &mut callisto.retro_tangent, &mut track);
                            unit(
                                ui,
                                "Terminator length",
                                &mut callisto.terminator_length,
                                &mut track,
                            );
                        });
                });

            if ui
                .add_enabled(
                    !callisto.is_default(),
                    egui::Button::new("Reset to Lambert + GGX"),
                )
                .clicked()
            {
                reset = true;
            }
        });
    if reset {
        *callisto = Callisto::default();
    }
    (changed || reset, released)
}

fn intensity(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    track: &mut impl FnMut(egui::Response),
) -> egui::Response {
    let label = ui.label(label);
    track(ui.add(crate::numeric::drag(value).speed(0.01).range(range)));
    ui.end_row();
    label
}

fn unit(ui: &mut egui::Ui, label: &str, value: &mut f32, track: &mut impl FnMut(egui::Response)) {
    ui.label(label);
    track(ui.add(egui::Slider::new(value, 0.0..=1.0)));
    ui.end_row();
}

fn tint(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut [f32; 3],
    track: &mut impl FnMut(egui::Response),
) {
    ui.label(label);
    track(ui.color_edit_button_rgb(value));
    ui.end_row();
}
