//! Systems panel — what runs each frame, and what is switched off.
//!
//! Two groups, because they are two different decisions: switching off
//! an engine system is an experiment and can break a frame, while
//! switching off a project system is ordinary gameplay control.
//!
//! 🔴 No GPU group. Nothing implements `GpuSystem` outside tests, so the
//! group would be permanently empty — a question nobody can answer by
//! looking at the screen. It arrives with #392.

use kooch_remote::protocol::SystemEntry;

use crate::actions::EditorAction;

/// Content of the "Systems" tab.
pub(crate) fn draw_systems_content(
    ui: &mut egui::Ui,
    systems: &[SystemEntry],
    actions: &mut Vec<EditorAction>,
) {
    if systems.is_empty() {
        // Says which of the two silences this is. "No systems" would be
        // wrong in both cases and unactionable in either.
        ui.weak("Nothing has reported its systems yet.");
        ui.weak("Open a project, or wait for it to connect.");
        return;
    }

    let off = systems.iter().filter(|system| !system.enabled).count();
    ui.horizontal(|ui| {
        ui.label(format!("{} systems", systems.len()));
        if off > 0 {
            // 🔴 A profile taken with a system switched off is a
            // measurement of an engine nobody ships, and this panel is
            // the only thing that knows.
            ui.colored_label(SWITCHED_OFF, format!("· {off} switched off"));
            if ui.button("Enable all").clicked() {
                actions.extend(switched_off(systems).map(|system| {
                    EditorAction::SetSystemEnabled {
                        name: system.name.clone(),
                        nth: system.nth,
                        enabled: true,
                    }
                }));
            }
        }
    });
    ui.separator();

    egui::ScrollArea::vertical()
        .id_salt("systems_list")
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            draw_group(ui, "Project", true, systems, actions);
            draw_group(ui, "Engine", false, systems, actions);
        });
}

/// The systems "Enable all" has to reach, and only those.
fn switched_off(systems: &[SystemEntry]) -> impl Iterator<Item = &SystemEntry> {
    systems.iter().filter(|system| !system.enabled)
}

/// The amber the dirty-scene marker uses, and for the same reason:
/// something here is out of step with what a build would do.
const SWITCHED_OFF: egui::Color32 = egui::Color32::from_rgb(210, 150, 60);

/// One half of the split, its rows grouped under the stage they run in.
fn draw_group(
    ui: &mut egui::Ui,
    label: &str,
    project: bool,
    systems: &[SystemEntry],
    actions: &mut Vec<EditorAction>,
) {
    let mine: Vec<&SystemEntry> = systems
        .iter()
        .filter(|system| system.project == project)
        .collect();
    if mine.is_empty() {
        return;
    }

    egui::CollapsingHeader::new(format!("{label} ({})", mine.len()))
        .default_open(project)
        .show(ui, |ui| {
            // The list arrives in run order, so a stage header is due
            // wherever the stage changes rather than after a sort — which
            // would have to know the run order all over again.
            let mut stage = "";
            for system in mine {
                if system.stage != stage {
                    stage = &system.stage;
                    ui.add_space(2.0);
                    ui.weak(stage);
                }
                draw_row(ui, system, actions);
            }
        });
}

/// One system: a checkbox, its short name, and its full path on hover.
fn draw_row(ui: &mut egui::Ui, system: &SystemEntry, actions: &mut Vec<EditorAction>) {
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        let mut enabled = system.enabled;
        // ⚠️ Disabled for a GPU system. Skipping one takes it out of the
        // batch that shares a command encoder, which changes how the
        // frame is RECORDED and not just what runs (#392).
        let toggle = ui
            .add_enabled(!system.gpu, egui::Checkbox::new(&mut enabled, ""))
            .on_disabled_hover_text("A GPU system shares an encoder with its neighbours");
        if toggle.changed() {
            actions.push(EditorAction::SetSystemEnabled {
                name: system.name.clone(),
                nth: system.nth,
                enabled,
            });
        }

        let name = match system.nth {
            // Two anonymous closures in one module share a `type_name`,
            // so the occurrence is the only thing telling them apart.
            0 => system.short.clone(),
            nth => format!("{} #{nth}", system.short),
        };
        let text = match system.enabled {
            true => egui::RichText::new(name),
            false => egui::RichText::new(name)
                .color(SWITCHED_OFF)
                .strikethrough(),
        };
        ui.label(text).on_hover_text(&system.name);
    });
}

#[cfg(test)]
mod tests;
