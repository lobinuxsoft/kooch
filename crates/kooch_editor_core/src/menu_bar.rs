//! Editor menu bar drawing.

use egui_dock::DockState;

use crate::actions::EditorAction;
use crate::icons;
use crate::remote_session::ConnectionState;
use crate::state::{ALL_TABS, EditorTab, dock_has_tab};

/// What the Edit menu needs beyond the undo stack: what a chord would act on.
pub(crate) struct EditMenu<'a> {
    pub selected: &'a [kooch_ecs::entity::Entity],
    pub clipboard_has_entities: bool,
    /// What a Ctrl+Z would reach, or `None` over a panel that edits nothing. Named in the entry so
    /// the menu says *which* history — "Undo Set intensity (this prefab)" is the difference between
    /// trusting the chord and testing it.
    pub document: Option<&'a crate::history::Document>,
}

/// Draws the Edit menu: every chord, whether it can run, and why.
fn draw_edit_menu(
    ui: &mut egui::Ui,
    actions: &mut Vec<EditorAction>,
    edit: &EditMenu<'_>,
    can_undo: bool,
    can_redo: bool,
    undo_desc: Option<&str>,
    redo_desc: Option<&str>,
) {
    use crate::shortcuts::{ALL, EditChord, actions_for};

    for chord in ALL {
        // Undo and Redo name the step they would take — "Undo Duplicate
        // Entity" — which is the whole reason the history keeps labels.
        let scope = edit
            .document
            .map(|document| format!(" ({})", document.describe()))
            .unwrap_or_default();
        let text = match (chord, undo_desc, redo_desc) {
            (EditChord::Undo, Some(desc), _) => {
                format!("Undo {desc}{scope}  {}", chord.chord())
            }
            (EditChord::Redo, _, Some(desc)) => {
                format!("Redo {desc}{scope}  {}", chord.chord())
            }
            _ => chord.menu_text(),
        };
        let enabled = match chord {
            EditChord::Undo => can_undo && edit.document.is_some(),
            EditChord::Redo => can_redo && edit.document.is_some(),
            EditChord::Duplicate | EditChord::Copy => !edit.selected.is_empty(),
            EditChord::Paste => edit.clipboard_has_entities,
        };
        if chord == EditChord::Duplicate {
            ui.separator();
        }
        if ui
            .add_enabled(enabled, egui::Button::new(text))
            .on_hover_text(chord.tooltip())
            .clicked()
        {
            actions.extend(actions_for(chord, edit.selected, edit.document));
            ui.close();
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_menu_bar(
    ui: &mut egui::Ui,
    dock_state: &mut DockState<EditorTab>,
    windows: &mut crate::os_windows::OsWindows,
    actions: &mut Vec<EditorAction>,
    is_playing: bool,
    remote: Option<ConnectionState>,
    remote_stale: Option<&str>,
    scripts_behind: bool,
    can_undo: bool,
    can_redo: bool,
    undo_desc: Option<&str>,
    redo_desc: Option<&str>,
    _ide_command: Option<&str>,
    edit: EditMenu<'_>,
) {
    // 🔴 The chords are read after the dock draws, in `run_editor_ui`, not here.

    // `Panel::top` in egui 0.35: `SidePanel` and `TopBottomPanel` were
    // unified into one `Panel` type (egui #5659).
    egui::Panel::top("editor_menu").show(ui, |ui| {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Save Scene...").clicked() {
                    actions.push(EditorAction::SaveScene);
                    ui.close();
                }
                if ui.button("Open Scene...").clicked() {
                    actions.push(EditorAction::OpenScene { path: None });
                    ui.close();
                }
                // Disabled while driving a project: the world here is a mirror of the project's,
                // and an additive load would spawn entities that exist only on this side.
                let mirroring = remote == Some(ConnectionState::Connected);
                let additive = ui
                    .add_enabled(!mirroring, egui::Button::new("Open Scene Additive..."))
                    .on_hover_text(if mirroring {
                        "Unavailable while a project is open — the world \
                         shown here mirrors the project, and a scene loaded \
                         on this side would not exist in it"
                    } else {
                        "Load a scene beside the ones already open"
                    });
                if additive.clicked() {
                    actions.push(EditorAction::OpenSceneAdditive { path: None });
                    ui.close();
                }
                ui.separator();
                if ui
                    .button("Clean Project")
                    .on_hover_text(
                        "Run cargo clean: deletes the build output and nothing else. \
                         The project has to be rebuilt afterwards.",
                    )
                    .clicked()
                {
                    actions.push(EditorAction::CleanProject);
                    ui.close();
                }
                if ui.button("Close Project").clicked() {
                    actions.push(EditorAction::CloseProject);
                    ui.close();
                }
            });
            ui.menu_button("Edit", |ui| {
                draw_edit_menu(ui, actions, &edit, can_undo, can_redo, undo_desc, redo_desc);
            });
            ui.menu_button("Window", |ui| {
                for &tab in ALL_TABS {
                    let detached = windows.detached.iter().any(|d| d.tab == tab);
                    let is_open = detached || dock_has_tab(dock_state, &tab);
                    if ui.selectable_label(is_open, tab.label()).clicked() {
                        if detached {
                            crate::os_windows::close(windows, tab);
                        } else if is_open {
                            dock_state.retain_tabs(|t| *t != tab);
                        } else {
                            dock_state.add_window(vec![tab]);
                        }
                        ui.close();
                    }
                }
            });
            // A button, not a menu with a form in it. An egui menu closes on any click it does not
            // consume, so a text field inside one shuts the menu the moment it is clicked — which
            // made the IDE path impossible to type.
            if ui.button("Settings").clicked() {
                ui.data_mut(|d| d.insert_temp(settings_open_id(ui.ctx()), true));
            }

            // Centre the transport controls in the remaining space.
            let button_width = 70.0;
            let spacing = ui.spacing().item_spacing.x;
            let widths = [button_width, button_width];
            let total_buttons: f32 =
                widths.iter().sum::<f32>() + spacing * (widths.len() - 1) as f32;
            // 🔴 Centred in the BAR, not in what is left of it.
            let bar = ui.max_rect();
            let spent = ui.cursor().left() - bar.left();
            let offset = (bar.width() - total_buttons) * 0.5 - spent;
            if offset > 0.0 {
                ui.add_space(offset);
            }

            if ui
                .add_enabled(
                    !is_playing,
                    egui::Button::new(format!("{} Play", icons::PLAY))
                        .min_size(egui::vec2(button_width, 0.0)),
                )
                .clicked()
            {
                actions.push(EditorAction::Play);
            }
            if ui
                .add_enabled(
                    is_playing,
                    egui::Button::new(format!("{} Stop", icons::STOP))
                        .min_size(egui::vec2(button_width, 0.0)),
                )
                .clicked()
            {
                actions.push(EditorAction::Stop);
            }

            // Right-aligned, and in a right-to-left layout the first
            // thing drawn is the furthest right — so the version sits at
            // the corner and the remote status stays beside it.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // 🔴 Which editor is this.
                ui.label(
                    egui::RichText::new(crate::engine_vendor::editor_engine_version())
                        .weak()
                        .small(),
                )
                .on_hover_text(
                    "Editor version. The project builds against the engine vendored for \
                     it, which the Project Manager shows per project.",
                );
                // Remote status: a project build takes long enough that a
                // silent editor reads as a hang.
                if let Some(remote) = remote {
                    ui.separator();
                    draw_remote_status(ui, remote, remote_stale, scripts_behind, actions);
                }
            });
        });
    });
}

/// Draws the remote session indicator and its rebuild control.
fn draw_remote_status(
    ui: &mut egui::Ui,
    remote: ConnectionState,
    stale: Option<&str>,
    // `true` when `src/` moved since the last build, so the project is
    // running code the author has already changed. Drawn here because
    // this is the button that fixes it.
    behind: bool,
    actions: &mut Vec<EditorAction>,
) {
    let stale_hover;
    let (icon, text, color, hover) = match (remote, stale) {
        (ConnectionState::Connected, Some(reason)) => {
            stale_hover = format!(
                "The project stopped answering with a readable world, so this is the \
                 last one the editor could read — edits from here may not land.\n\n{reason}",
            );
            (
                // The same glyph the Inspector uses for a warning; the
                // icon font has no triangle of its own.
                "\u{26a0}",
                "Stale",
                egui::Color32::from_rgb(210, 150, 60),
                stale_hover.as_str(),
            )
        }
        _ => remote_status_look(remote),
    };
    // Right-to-left layout: this lands to the left of the status text.
    if ui
        .add_enabled(
            remote != ConnectionState::Connecting,
            // 🔴 The only button that does this now.
            egui::Button::new(rebuild_label(ui, behind)),
        )
        .on_hover_text(match behind {
            true => {
                "A source file changed since the last build, so the project is \
                 running code you have already edited. The editor reads components, \
                 fields and defaults out of the compiled binary, so this is what \
                 shows them.\n\nYour scene and selection survive it."
            }
            false => {
                "Recompiles the project and restarts it. This is what picks up a \
                 changed component or system.\n\nYour scene and selection survive it."
            }
        })
        .clicked()
    {
        actions.push(EditorAction::RebuildAndRun);
    }
    ui.label(egui::RichText::new(format!("{icon} {text}")).color(color))
        .on_hover_text(hover);
}

/// The rebuild button's face, pulsing while the build is behind.
fn rebuild_label(ui: &egui::Ui, behind: bool) -> egui::RichText {
    let text = format!("{} Rebuild & Run", icons::PACKAGE);
    if !behind {
        return egui::RichText::new(text);
    }
    let phase = (ui.input(|i| i.time) * 1.6).fract() as f32;
    let lit = 1.0 - (phase * 2.0 - 1.0).abs();
    // Orange, and swinging further than the old amber did. Not red:
    // red is how this editor says something is broken, and a build that
    // is merely behind is not.
    let colour = egui::Color32::from_rgb(255, 120 + (110.0 * lit) as u8, 30);
    ui.ctx().request_repaint();
    egui::RichText::new(text).color(colour).strong()
}

/// How each handshake state reads, before staleness is taken into account.
fn remote_status_look(
    remote: ConnectionState,
) -> (&'static str, &'static str, egui::Color32, &'static str) {
    match remote {
        ConnectionState::Connecting => (
            icons::GEAR,
            "Connecting",
            egui::Color32::from_rgb(210, 180, 90),
            "Building and starting the project. The world appears once it answers.",
        ),
        ConnectionState::Connected => (
            icons::ROCKET,
            "Remote",
            egui::Color32::from_rgb(100, 200, 100),
            "Editing the project's live world. Edits are applied by the project, not here.",
        ),
        ConnectionState::Failed => (
            icons::X,
            "Disconnected",
            egui::Color32::from_rgb(200, 80, 80),
            "The project exited before answering. Check its build output in the terminal.",
        ),
    }
}

mod settings;

pub(crate) use settings::{draw_preflight_window, draw_settings_window};
use settings::*;
