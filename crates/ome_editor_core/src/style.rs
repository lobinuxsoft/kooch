//! Font and style configuration for the editor UI.

use std::sync::Arc;

pub(crate) fn configure_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    fonts.font_data.insert(
        "firacode".to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../assets/fonts/FiraCode-Regular.ttf"
        ))),
    );

    fonts.font_data.insert(
        "phosphor".to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../assets/fonts/Phosphor.ttf"
        ))),
    );

    if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        family.insert(0, "firacode".to_owned());
        family.push("phosphor".to_owned());
    }

    if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
        family.insert(0, "firacode".to_owned());
        family.push("phosphor".to_owned());
    }

    ctx.set_fonts(fonts);
}

pub(crate) fn configure_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals.window_rounding = egui::Rounding::same(6.0);
    style.visuals.menu_rounding = egui::Rounding::same(4.0);
    style.spacing.item_spacing = egui::vec2(6.0, 4.0);
    ctx.set_style(style);
}
