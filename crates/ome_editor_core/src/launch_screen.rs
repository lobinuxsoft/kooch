//! Launch screen UI for the editor.
//!
//! Renders the initial project selection screen with recent projects,
//! "New Project" form, "Open Project" button, and the launcher
//! compilation/output overlay.

use std::path::PathBuf;

use crate::icons;
use crate::project_state::{LauncherStatus, ProjectState};

/// Actions that the launch screen can produce.
pub enum LaunchAction {
    OpenProject(PathBuf),
    CreateProject { name: String, parent_path: PathBuf },
    RemoveRecent(PathBuf),
    LaunchProject(PathBuf),
    CancelLaunch,
}

/// Draws the full launch screen UI. Returns a list of actions to apply.
pub fn draw_launch_screen(ctx: &egui::Context, project_state: &mut ProjectState) -> Vec<LaunchAction> {
    let mut actions = Vec::new();

    // If a launcher process is active, show the launcher overlay instead.
    if project_state.launcher_process.is_some() {
        draw_launcher_overlay(ctx, project_state, &mut actions);
        return actions;
    }

    egui::CentralPanel::default().show(ctx, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(40.0);
            ui.heading(
                egui::RichText::new("Oh My Engine")
                    .size(32.0)
                    .strong(),
            );
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new("Project Manager")
                    .size(14.0)
                    .weak(),
            );
            ui.add_space(24.0);
        });

        let panel_width = (ui.available_width() - 40.0).min(600.0);
        ui.vertical_centered(|ui| {
            ui.set_max_width(panel_width);

            // --- Action buttons row ---
            ui.horizontal(|ui| {
                if ui
                    .button(format!("{} New Project", icons::FOLDER_PLUS))
                    .clicked()
                {
                    project_state.show_new_project_form = !project_state.show_new_project_form;
                    project_state.new_project_form.error = None;
                }

                if ui
                    .button(format!("{} Open Project", icons::FOLDER_OPEN))
                    .clicked()
                {
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                        // Always open in-process — running the project crate
                        // is reserved for the editor's Play action.
                        actions.push(LaunchAction::OpenProject(path));
                    }
                }
            });

            ui.add_space(8.0);

            // --- New Project form (inline) ---
            if project_state.show_new_project_form {
                draw_new_project_form(ui, project_state, &mut actions);
                ui.add_space(8.0);
            }

            // --- Recent Projects ---
            ui.separator();
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("Recent Projects")
                    .size(16.0)
                    .strong(),
            );
            ui.add_space(4.0);

            let recent = project_state.editor_config.recent_projects.clone();
            if recent.is_empty() {
                ui.label(
                    egui::RichText::new("No recent projects")
                        .weak()
                        .italics(),
                );
            } else {
                for entry in &recent {
                    let exists = entry.path.join("project.ome").exists();
                    ui.horizontal(|ui| {
                        // Clickable project entry.
                        let label_text = if exists {
                            format!("{} {}", icons::CUBE, entry.name)
                        } else {
                            format!("{} {} (missing)", icons::CUBE, entry.name)
                        };

                        let label = if exists {
                            egui::RichText::new(&label_text)
                        } else {
                            egui::RichText::new(&label_text)
                                .weak()
                                .strikethrough()
                        };

                        let response = ui.add_enabled(
                            exists,
                            egui::Button::new(label).frame(false),
                        );

                        if response.clicked() {
                            actions.push(LaunchAction::OpenProject(entry.path.clone()));
                        }

                        if response.hovered() && exists {
                            response.on_hover_text(entry.path.display().to_string());
                        }

                        // Path display.
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                // Remove button.
                                if ui
                                    .small_button(egui::RichText::new(icons::X).weak())
                                    .on_hover_text("Remove from recent")
                                    .clicked()
                                {
                                    actions.push(LaunchAction::RemoveRecent(entry.path.clone()));
                                }

                                ui.label(
                                    egui::RichText::new(entry.path.display().to_string())
                                        .weak()
                                        .small(),
                                );
                            },
                        );
                    });
                }
            }
        });
    });

    actions
}

/// Draws the inline "New Project" form.
fn draw_new_project_form(
    ui: &mut egui::Ui,
    project_state: &mut ProjectState,
    actions: &mut Vec<LaunchAction>,
) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.label(egui::RichText::new("New Project").strong());
        ui.add_space(4.0);

        egui::Grid::new("new_project_grid")
            .num_columns(2)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                ui.label("Name:");
                ui.text_edit_singleline(&mut project_state.new_project_form.name);
                ui.end_row();

                ui.label("Location:");
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut project_state.new_project_form.parent_path);
                    if ui.button("Browse...").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            project_state.new_project_form.parent_path =
                                path.display().to_string();
                        }
                    }
                });
                ui.end_row();
            });

        if let Some(ref err) = project_state.new_project_form.error {
            ui.colored_label(egui::Color32::RED, err);
        }

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let name = project_state.new_project_form.name.trim().to_owned();
            let parent = project_state.new_project_form.parent_path.trim().to_owned();
            let can_create = !name.is_empty() && !parent.is_empty();

            if ui.add_enabled(can_create, egui::Button::new("Create")).clicked() {
                actions.push(LaunchAction::CreateProject {
                    name,
                    parent_path: PathBuf::from(parent),
                });
            }
            if ui.button("Cancel").clicked() {
                project_state.show_new_project_form = false;
                project_state.new_project_form = Default::default();
            }
        });
    });
}

/// Draws the launcher compilation/running overlay.
fn draw_launcher_overlay(
    ctx: &egui::Context,
    project_state: &mut ProjectState,
    actions: &mut Vec<LaunchAction>,
) {
    let status = project_state
        .launcher_status()
        .cloned()
        .unwrap_or(LauncherStatus::Compiling);

    egui::CentralPanel::default().show(ctx, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(40.0);
            ui.heading(
                egui::RichText::new("Oh My Engine")
                    .size(32.0)
                    .strong(),
            );
            ui.add_space(16.0);

            // Status indicator.
            match &status {
                LauncherStatus::Compiling => {
                    ui.label(
                        egui::RichText::new(format!("{} Compiling...", icons::GEAR))
                            .size(18.0),
                    );
                }
                LauncherStatus::Launched => {
                    ui.label(
                        egui::RichText::new(format!("{} Launching project...", icons::ROCKET))
                            .size(18.0)
                            .color(egui::Color32::from_rgb(100, 200, 100)),
                    );
                }
                LauncherStatus::Failed(msg) => {
                    ui.label(
                        egui::RichText::new(format!("{} Failed", icons::X))
                            .size(18.0)
                            .color(egui::Color32::from_rgb(200, 80, 80)),
                    );
                    ui.label(
                        egui::RichText::new(msg).weak().small(),
                    );
                }
                LauncherStatus::Exited => {
                    ui.label(
                        egui::RichText::new("Process exited")
                            .size(18.0)
                            .weak(),
                    );
                }
            }
            ui.add_space(12.0);
        });

        // Output console.
        let panel_width = (ui.available_width() - 40.0).min(800.0);
        ui.vertical_centered(|ui| {
            ui.set_max_width(panel_width);

            ui.label(
                egui::RichText::new(format!("{} Output", icons::TERMINAL)).strong(),
            );
            ui.add_space(4.0);

            let output_lines = &project_state.launcher_output;
            let frame = egui::Frame::group(ui.style())
                .fill(egui::Color32::from_rgb(20, 20, 20));
            frame.show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for line in output_lines {
                            ui.label(
                                egui::RichText::new(line)
                                    .monospace()
                                    .size(11.0)
                                    .color(egui::Color32::from_rgb(200, 200, 200)),
                            );
                        }
                    });
            });

            ui.add_space(12.0);

            // Action buttons.
            ui.horizontal(|ui| {
                match &status {
                    LauncherStatus::Compiling => {
                        if ui.button(format!("{} Cancel", icons::X)).clicked() {
                            actions.push(LaunchAction::CancelLaunch);
                        }
                    }
                    LauncherStatus::Launched => {
                        // App will exit shortly, nothing to show.
                    }
                    LauncherStatus::Failed(_) | LauncherStatus::Exited => {
                        if ui
                            .button(format!("{} Back", icons::ARROW_LEFT))
                            .clicked()
                        {
                            actions.push(LaunchAction::CancelLaunch);
                        }
                    }
                }
            });
        });
    });

    // Request repaint while the launcher is active for live output.
    ctx.request_repaint();
}
