//! Components panel — lists all registered component types.

use crate::drag_drop::DraggedComponent;
use crate::icons;
use crate::state::ComponentTypeInfo;

/// Content of the "Components" tab — lists all registered component
/// types. Each row is a drag source: dragging a row onto a World
/// entity row or into the Inspector adds that component to the drop
/// target. See #209.
pub(crate) fn draw_components_content(ui: &mut egui::Ui, component_types: &[ComponentTypeInfo]) {
    let reflected = component_types.iter().filter(|c| c.has_reflection).count();
    ui.label(format!(
        "{} component types ({} with reflection)",
        component_types.len(),
        reflected,
    ));
    ui.weak("drag a row onto an entity to add the component");
    ui.separator();

    if component_types.is_empty() {
        ui.weak("(none)");
        return;
    }

    egui::ScrollArea::vertical().show(ui, |ui| {
        for comp in component_types {
            let drag_id = egui::Id::new(("drag_component_row", comp.type_id));
            ui.dnd_drag_source(drag_id, DraggedComponent(comp.type_id), |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!("{} {}", icons::PUZZLE_PIECE, &comp.short_name));
                    if comp.has_reflection {
                        ui.weak("(reflected)");
                    } else {
                        ui.weak("(opaque)");
                    }
                });
            });
        }
    });
}
