//! Archetypes panel — archetype browser.

use crate::icons;
use crate::state::ArchetypeDisplayInfo;

/// Content of the "Archetypes" tab.
pub(crate) fn draw_archetypes_content(ui: &mut egui::Ui, archetypes: &[ArchetypeDisplayInfo]) {
    let active = archetypes.iter().filter(|a| a.entity_count > 0).count();
    ui.label(format!(
        "{} archetypes ({} active, {} empty)",
        archetypes.len(),
        active,
        archetypes.len() - active,
    ));
    ui.separator();

    if archetypes.is_empty() {
        ui.weak("(none)");
        return;
    }

    egui::ScrollArea::vertical().show(ui, |ui| {
        for (i, arch) in archetypes.iter().enumerate() {
            let header = if arch.component_names.is_empty() {
                format!("{} Empty  —  {} entities", icons::STACK, arch.entity_count,)
            } else {
                format!(
                    "{} [{}]  —  {} entities",
                    icons::STACK,
                    arch.component_names.join(", "),
                    arch.entity_count,
                )
            };

            // Dim empty archetypes.
            let is_empty = arch.entity_count == 0;

            let id = ui.make_persistent_id(format!("arch_{}", i));
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false)
                .show_header(ui, |ui| {
                    if is_empty {
                        ui.weak(header);
                    } else {
                        ui.label(header);
                    }
                })
                .body(|ui| {
                    ui.label(format!("ID: {}", arch.id_short));
                    ui.label(format!("Entities: {}", arch.entity_count));
                    if arch.component_names.is_empty() {
                        ui.weak("No components (empty archetype)");
                    } else {
                        ui.label("Components:");
                        for name in &arch.component_names {
                            ui.label(format!("  {} {}", icons::PUZZLE_PIECE, name));
                        }
                    }
                });
        }
    });
}
