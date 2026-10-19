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
    let mut style = (*ctx.global_style()).clone();
    style.visuals.window_corner_radius = egui::CornerRadius::same(6);
    style.visuals.menu_corner_radius = egui::CornerRadius::same(4);
    style.spacing.item_spacing = egui::vec2(6.0, 4.0);
    // Disable text selection on labels globally. egui 0.34 defaults to selectable labels +
    // multi-widget text selection, which in an editor (a) has no functional purpose and (b) leaks
    // state across widgets after a DnD interaction ends with the pointer still registering as held.
    // 🔴 The Inspector paints its whole body with `widgets.inactive.bg_fill` (#1544), which is
    // the exact grey egui fills a button with: measured at 60 on both, every button in it was
    // camouflaged. Buttons and drag fields get a lighter fill so they read as controls on any
    // panel, dark or not.
    let widgets = &mut style.visuals.widgets;
    widgets.inactive.weak_bg_fill = egui::Color32::from_gray(96);
    widgets.hovered.weak_bg_fill = egui::Color32::from_gray(118);
    widgets.active.weak_bg_fill = egui::Color32::from_gray(140);
    style.interaction.selectable_labels = false;
    style.interaction.multi_widget_text_select = false;
    ctx.set_global_style(style);
}
