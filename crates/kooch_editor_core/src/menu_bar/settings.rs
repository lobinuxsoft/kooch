//! The Settings and preflight windows: engines, the DLSS SDK, the launch line and the install button.

use super::*;

/// The engines this machine has, which of them is in use, and a way to get rid of the rest.
pub(super) fn draw_installed_engines(
    ui: &mut egui::Ui,
    project_engine: Option<&str>,
    actions: &mut Vec<EditorAction>,
) {
    let installed = crate::engine_vendor::installed_engines();
    let editor_version = crate::engine_vendor::editor_engine_version();

    ui.label("Engines on this machine:");
    ui.add_space(4.0);

    if installed.is_empty() {
        ui.weak("None yet — one is installed the first time a project is opened.");
        return;
    }

    for engine in &installed {
        ui.horizontal(|ui| {
            let in_use = Some(engine.version.as_str()) == project_engine;
            let is_editors = engine.version == editor_version;

            ui.monospace(&engine.version);
            if is_editors {
                ui.weak("(this editor)");
            }
            if in_use {
                ui.weak("(this project)");
            }

            // 🔴 Neither of those two may be removed. The editor's is what the next project to open
            // is pointed at, and the project's is what it builds against — deleting either leaves a
            // manifest naming a directory that is not there.
            if !is_editors && !in_use && ui.button("Remove").clicked() {
                actions.push(EditorAction::RemoveEngine(engine.version.clone()));
            }
        });
        ui.weak(engine.path.display().to_string());
        ui.add_space(4.0);
    }
}

/// The DLSS SDK: whether this machine has it, and the one button that fetches it.
pub(super) fn draw_dlss_sdk(ui: &mut egui::Ui, install: &mut crate::dlss_sdk::SdkInstall) {
    use crate::dlss_sdk::{LICENSE, SdkState, VERSION};

    install.poll();
    ui.label(egui::RichText::new(format!("DLSS SDK {VERSION}")).strong());

    match install.state().clone() {
        SdkState::Installed(dir) => {
            ui.weak(format!("Installed: {}", dir.display()));
            ui.weak(
                "Set DLSS_SDK to that path in Launch environment and the game finds the \
                 runtime without copying it.",
            );
            return;
        }
        SdkState::Fetching(what) => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.weak(what);
            });
            return;
        }
        SdkState::Nowhere => {
            ui.weak("No data directory on this platform to put it in.");
            return;
        }
        SdkState::Failed(problem) => {
            ui.colored_label(egui::Color32::from_rgb(220, 120, 120), problem);
        }
        SdkState::Missing(dir) => {
            ui.weak(format!("Not installed. It would go in {}", dir.display()));
        }
    }

    ui.weak(
        "Downloaded from NVIDIA, never from us — their licence forbids redistributing \
         the SDK. It is ~700 MB and it does not enable DLSS on its own; nothing in the \
         engine calls it yet.",
    );
    ui.hyperlink_to("Read the licence", LICENSE);
    ui.checkbox(&mut install.accepted, "I accept NVIDIA's SDK licence");

    let can = install.can_fetch();
    if ui
        .add_enabled(can, egui::Button::new("Download the SDK"))
        .on_disabled_hover_text("Accept the licence first.")
        .clicked()
    {
        install.fetch();
    }
}

/// The open project's launch environment, for the Play button.
pub(super) fn draw_launch_env(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    id: egui::Id,
    current: &str,
    actions: &mut Vec<EditorAction>,
) {
    let buffer_id = id.with("launch_env_buffer");
    let mut buf: String = ctx
        .data(|d| d.get_temp::<String>(buffer_id))
        .unwrap_or_else(|| current.to_owned());

    ui.label("Launch environment — variables the Play button gives this project's game:");
    if ui.text_edit_singleline(&mut buf).changed() {
        ctx.data_mut(|d| d.insert_temp(buffer_id, buf.clone()));
    }
    ui.weak(
        "Whitespace-separated KEY=VALUE, e.g. 'KOOCH_SHADING_PAD=4 \
         KOOCH_FRAME_METRICS=log'. No quotes: a value with a space in it \
         needs a shell's rules, and these variables are single words. \
         Stored against this project alone, in the editor's config rather \
         than in the project, because a measurement does not belong in a \
         repository.",
    );

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui.button("Apply").clicked() {
            actions.push(EditorAction::SetLaunchEnv {
                value: buf.trim().to_owned(),
            });
            ctx.data_mut(|d| d.remove::<String>(buffer_id));
        }
        if ui.button("Clear").clicked() {
            actions.push(EditorAction::SetLaunchEnv {
                value: String::new(),
            });
            buf.clear();
            ctx.data_mut(|d| d.remove::<String>(buffer_id));
        }
    });

    ui.add_space(4.0);
    match current.trim().is_empty() {
        true => ui.weak("In use: nothing — the game inherits the editor's environment."),
        false => ui.weak(format!("In use: {current}")),
    };
    ui.weak(
        "KOOCH_ENGINE_ROOT, KOOCH_PROJECT_ROOT and KOOCH_LOG_FORMAT are the \
         editor's and override anything typed here — the Console cannot read \
         the game's output without the last one.",
    );
}

/// What this machine is missing before a project can build.
pub(crate) fn draw_preflight_window(
    ctx: &egui::Context,
    report: &crate::preflight::Report,
    blocked: Option<&crate::install::Refusal>,
    installing: Option<&crate::install::Progress>,
    actions: &mut Vec<EditorAction>,
) {
    if report.is_quiet() && installing.is_none() {
        return;
    }
    let id = egui::Id::new("kooch_preflight_window_open");
    let mut open = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(true);
    if !open {
        return;
    }

    let title = match report.is_ready() {
        true => "This machine could build faster",
        false => "This machine cannot build a project yet",
    };
    egui::Window::new(title)
        .open(&mut open)
        .resizable(true)
        .default_width(520.0)
        .collapsible(false)
        .show(ctx, |ui| {
            if !report.missing.is_empty() {
                ui.label(
                    "A project made with this editor compiles the engine, so it needs a \
                     Rust toolchain and the system libraries the engine links against. \
                     These are missing:",
                );
            }
            ui.add_space(6.0);
            for requirement in &report.missing {
                ui.label(egui::RichText::new(requirement.name).strong());
                ui.weak(requirement.why);
                // The hint is where it comes from; the block below is
                // how. Shown only when there is no block — otherwise it
                // is a second answer to a question already answered.
                if !requirement.hint.is_empty() && report.command().is_none() {
                    ui.weak(format!("    {}", requirement.hint));
                }
                ui.add_space(4.0);
            }

            // 🔴 The optional half is listed apart and last. Mixing "you cannot build without this"
            // with "this would be faster" is how a list gets read diagonally — which is exactly how
            // three packages once got installed for nothing.
            if !report.wanted.is_empty() {
                ui.separator();
                ui.add_space(4.0);
                ui.label(egui::RichText::new("Not required — installed in the same step:").weak());
                ui.add_space(4.0);
                for requirement in &report.wanted {
                    ui.label(egui::RichText::new(requirement.name).strong());
                    ui.weak(requirement.why);
                    ui.add_space(4.0);
                }
            }

            match report.command() {
                Some(command) => {
                    ui.separator();
                    ui.add_space(4.0);
                    match installing {
                        Some(installing) => draw_install_progress(ui, installing),
                        None => draw_install_button(ui, report, blocked, actions),
                    }
                    ui.add_space(6.0);
                    ui.label("Or paste this whole block yourself:");
                    ui.code(&command);
                    if ui.button(format!("{} Copy", icons::COPY)).clicked() {
                        ctx.copy_text(command);
                    }
                }
                // Named without a command rather than given a wrong one:
                // an unrecognised package manager, or a requirement no
                // package manager provides.
                None => {
                    ui.add_space(4.0);
                    ui.weak(
                        "No package-manager command for this machine — install the above \
                         the way this system installs things.",
                    );
                }
            }
        });

    ctx.data_mut(|d| d.insert_temp(id, open));
}

/// What the installer is saying, while it says it.
pub(super) fn draw_install_progress(ui: &mut egui::Ui, installing: &crate::install::Progress) {
    ui.label(egui::RichText::new(installing.status).strong());
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .max_height(180.0)
        // Pinned to the bottom: the interesting line is the last one.
        .stick_to_bottom(true)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for line in &installing.lines {
                ui.label(egui::RichText::new(line).monospace().weak());
            }
        });
    if installing.running {
        // The only place in this window that asks for one, and only
        // while there is something moving to show.
        ui.ctx().request_repaint();
    }
}

/// The install control, and the sentence that says what it will do.
pub(super) fn draw_install_button(
    ui: &mut egui::Ui,
    report: &crate::preflight::Report,
    blocked: Option<&crate::install::Refusal>,
    actions: &mut Vec<EditorAction>,
) {
    let label = match report.reboots() {
        true => "Install and restart this machine",
        false => "Install these now",
    };
    let response = ui.add_enabled(
        blocked.is_none(),
        egui::Button::new(format!("{} {label}", icons::PACKAGE)),
    );
    match blocked {
        Some(reason) => {
            response.on_disabled_hover_text(reason.to_string());
        }
        None => {
            if response.clicked() {
                actions.push(EditorAction::InstallRequirements);
            }
        }
    }
    // Said in the window, not only in a hover: a restart is not a
    // footnote, and a hover is not read by someone already reaching for
    // the button.
    if report.reboots() {
        ui.label(
            egui::RichText::new(
                "Your system installs packages into a new image, so this restarts the \
                 computer when it finishes. Nothing is restarted if the install fails.",
            )
            .weak(),
        );
    }
    if report.needs_rust() {
        ui.label(
            egui::RichText::new(
                "Rust is not included: rustup installs into your own home, and this \
                 cannot run it for you. Use the line below after the restart.",
            )
            .weak(),
        );
    }
}

/// Where the Settings window's open flag lives.
pub(super) fn settings_open_id(ctx: &egui::Context) -> egui::Id {
    let _ = ctx;
    egui::Id::new("kooch_settings_window_open")
}

/// The Settings window: editor preferences, and the two things about the open project that belong
/// beside them.
pub(crate) fn draw_settings_window(
    ctx: &egui::Context,
    actions: &mut Vec<EditorAction>,
    ide_command: Option<&str>,
    project_engine: Option<&str>,
    launch_env: Option<&str>,
    dlss: &mut crate::dlss_sdk::SdkInstall,
) {
    let id = settings_open_id(ctx);
    let mut open = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    if !open {
        return;
    }

    let buffer_id = id.with("ide_cmd_buffer");
    let mut buf: String = ctx
        .data(|d| d.get_temp::<String>(buffer_id))
        .unwrap_or_else(|| ide_command.unwrap_or_default().to_owned());

    egui::Window::new("Settings")
        .open(&mut open)
        .resizable(true)
        .default_width(460.0)
        .collapsible(false)
        .show(ctx, |ui| {
            ui.label("IDE command — opens the project folder, and the file if it can:");
            if ui.text_edit_singleline(&mut buf).changed() {
                ctx.data_mut(|d| d.insert_temp(buffer_id, buf.clone()));
            }
            ui.weak(
                "A full path is safest: an IDE installed by Flatpak, Homebrew or an \
                 AppImage is usually not on this process's PATH. Arguments are fine, \
                 e.g. 'flatpak run com.vscodium.codium'.",
            );

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Apply").clicked() {
                    let trimmed = buf.trim();
                    let command = (!trimmed.is_empty()).then(|| trimmed.to_owned());
                    actions.push(EditorAction::SetIdeCommand { command });
                    ctx.data_mut(|d| d.remove::<String>(buffer_id));
                }
                // Fills the box rather than applying, so what was found can
                // be read and edited before it is committed.
                if ui.button("Detect").clicked()
                    && let Some(found) = crate::actions::detected_ide_command()
                {
                    buf = found;
                    ctx.data_mut(|d| d.insert_temp(buffer_id, buf.clone()));
                }
                if ui.button("Clear").clicked() {
                    actions.push(EditorAction::SetIdeCommand { command: None });
                    buf.clear();
                    ctx.data_mut(|d| d.remove::<String>(buffer_id));
                }
            });

            ui.add_space(4.0);
            match ide_command {
                Some(current) => ui.weak(format!("In use: {current}")),
                None => ui.weak("In use: whatever the desktop says opens a source file."),
            };

            if let Some(current) = launch_env {
                ui.add_space(12.0);
                ui.separator();
                ui.add_space(6.0);
                draw_launch_env(ui, ctx, id, current, actions);
            }

            ui.add_space(12.0);
            ui.separator();
            ui.add_space(6.0);
            draw_dlss_sdk(ui, dlss);

            ui.add_space(12.0);
            ui.separator();
            ui.add_space(6.0);
            draw_installed_engines(ui, project_engine, actions);
        });

    ctx.data_mut(|d| d.insert_temp(id, open));
}
