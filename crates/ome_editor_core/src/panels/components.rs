//! Components panel — lists all registered component types.

use crate::icons;
use crate::state::ComponentTypeInfo;

/// Content of the "Components" tab — lists all registered component types.
pub(crate) fn draw_components_content(ui: &mut egui::Ui, component_types: &[ComponentTypeInfo]) {
    let reflected = component_types.iter().filter(|c| c.has_reflection).count();
    ui.label(format!(
        "{} component types ({} with reflection)",
        component_types.len(),
        reflected,
    ));
    ui.separator();

    if component_types.is_empty() {
        ui.weak("(none)");
        return;
    }

    egui::ScrollArea::vertical().show(ui, |ui| {
        for comp in component_types {
            ui.horizontal(|ui| {
                ui.label(format!("{} {}", icons::PUZZLE_PIECE, &comp.short_name));
                if comp.has_reflection {
                    ui.weak("(reflected)");
                } else {
                    ui.weak("(opaque)");
                }
            });
        }
    });
}
