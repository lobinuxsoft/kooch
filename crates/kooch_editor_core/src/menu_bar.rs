//! Editor menu bar drawing.

use egui_dock::DockState;

use crate::actions::EditorAction;
use crate::icons;
use crate::remote_session::ConnectionState;
use crate::state::{ALL_TABS, EditorTab, dock_has_tab};

/// What the Edit menu needs beyond the undo stack: what a chord would
/// act on.
///
/// A struct rather than two more positional arguments on a function that
/// already takes eleven.
pub(crate) struct EditMenu<'a> {
    pub selected: &'a [kooch_ecs::entity::Entity],
    pub clipboard_has_entities: bool,
    /// What a Ctrl+Z would reach, or `None` over a panel that edits
    /// nothing. Named in the entry so the menu says *which* history —
    /// "Undo Set intensity (this prefab)" is the difference between
    /// trusting the chord and testing it.
    pub document: Option<&'a crate::history::Document>,
}

/// Draws the Edit menu: every chord, whether it can run, and why.
///
/// Entries are **disabled, not hidden**, when they have nothing to act
/// on. A menu that reorders itself according to the selection is a menu
/// where the entry you are reaching for is somewhere else, and a greyed
/// Paste is how a user learns the clipboard is empty.
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
    // 🔴 The chords are read after the dock draws, in `run_editor_ui`,
    // not here. They are gated on which panel has focus, and the menu bar
    // draws before the dock has said which one that is — reading them
    // here meant reading last frame's answer, and made "the shortcut
    // works if you press it twice" a real behaviour.

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
                    actions.push(EditorAction::OpenScene);
                    ui.close();
                }
                // Disabled while driving a project: the world here is a
                // mirror of the project's, and an additive load would
                // spawn entities that exist only on this side. They would
                // be invisible in the game and every edit to them would be
                // dropped for not being in the mirror. Loading a second
                // scene into the project needs the project to do it — see
                // the issue linked from the tooltip.
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
                    actions.push(EditorAction::OpenSceneAdditive);
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
                    let is_open = dock_has_tab(dock_state, &tab);
                    if ui.selectable_label(is_open, tab.label()).clicked() {
                        if is_open {
                            dock_state.retain_tabs(|t| *t != tab);
                        } else {
                            dock_state.add_window(vec![tab]);
                        }
                        ui.close();
                    }
                }
            });
            // A button, not a menu with a form in it. An egui menu closes
            // on any click it does not consume, so a text field inside one
            // shuts the menu the moment it is clicked — which made the IDE
            // path impossible to type. A menu picks an action; a form
            // belongs in a window.
            if ui.button("Settings").clicked() {
                ui.data_mut(|d| d.insert_temp(settings_open_id(ui.ctx()), true));
            }

            // Centre the transport controls in the remaining space.
            //
            // 🔴 Counted, not hardcoded to two. It was `button_width * 2`
            // when Play and Stop were the only ones here, so adding a
            // third pushed the whole group off centre by half a button —
            // visible immediately, and the kind of arithmetic that goes
            // wrong again the next time one is added.
            let available = ui.available_width();
            let button_width = 70.0;
            let spacing = ui.spacing().item_spacing.x;
            let widths = [button_width, button_width, SYNC_WIDTH];
            let total_buttons: f32 =
                widths.iter().sum::<f32>() + spacing * (widths.len() - 1) as f32;
            let offset = (available - total_buttons) / 2.0;
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
            draw_script_sync(ui, scripts_behind, actions);

            // Right-aligned, and in a right-to-left layout the first
            // thing drawn is the furthest right — so the version sits at
            // the corner and the remote status stays beside it.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // 🔴 Which editor is this. The engine a project builds
                // against is vendored per version, so "same version,
                // different source" is a warning an author cannot act on
                // without knowing what version they are running — and it
                // is the warning that hid a stale engine for a whole
                // session.
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
                    draw_remote_status(ui, remote, remote_stale, actions);
                }
            });
        });
    });
}

/// What the code-sync control occupies, in both of its states. Shared so
/// the centring above and the button below cannot disagree about it.
const SYNC_WIDTH: f32 = 120.0;

/// The code-sync control: quiet while the generated registrations match
/// the project's build, pulsing once they do not.
///
/// 🔴 It does NOT mean "`registrations.rs` is stale". The poll rewrote
/// that file already — announcing it would announce something handled.
/// What is behind is the BUILD: the editor lists a project's components
/// out of its compiled dylib, so a system added ten seconds ago is in the
/// generated file and in no binary anywhere. That gap is invisible, it
/// produces no error, and the symptom is "I pressed Play and my system
/// did not run".
///
/// ⚠️ The pulse asks for a repaint per frame, which in an immediate-mode
/// UI is a real and permanent cost — so it runs ONLY while behind. In
/// the ordinary state the widget is static and asks for nothing.
fn draw_script_sync(ui: &mut egui::Ui, behind: bool, actions: &mut Vec<EditorAction>) {
    if !behind {
        if ui
            .add(
                egui::Button::new(format!("{} Code Sync", icons::ARROWS_CLOCKWISE))
                    .min_size(egui::vec2(SYNC_WIDTH, 0.0)),
            )
            .on_hover_text(
                "Rescans `src/` and rewrites the generated registrations. Runs on \
                 its own when a source file changes; this forces it.",
            )
            .clicked()
        {
            actions.push(EditorAction::RegisterScripts);
        }
        return;
    }

    // A triangle wave rather than a sine: the eye catches a hard edge,
    // and a sine spends most of its time near the middle where the
    // difference from the resting colour is smallest.
    let phase = (ui.input(|i| i.time) * 1.6).fract() as f32;
    let lit = 1.0 - (phase * 2.0 - 1.0).abs();
    let colour = egui::Color32::from_rgb(150 + (105.0 * lit) as u8, 110 + (80.0 * lit) as u8, 40);
    let clicked = ui
        .add(
            egui::Button::new(
                egui::RichText::new(format!("{} Code Sync", icons::ARROWS_CLOCKWISE)).color(colour),
            )
            .min_size(egui::vec2(SYNC_WIDTH, 0.0)),
        )
        .on_hover_text(
            "The generated registrations changed, so the project's compiled code is \
             behind its source — a component or system added since the last build \
             exists in the file and in no binary. Rebuild the project, then click \
             this to clear the notice.",
        )
        .clicked();
    // Only while it pulses. See the header.
    ui.ctx().request_repaint();
    if clicked {
        actions.push(EditorAction::AcknowledgeScriptSync);
    }
}

/// Draws the remote session indicator and its rebuild control.
///
/// `stale` is why the snapshot stopped tracking the project. It overrides
/// the connected look: the socket being up says nothing about whether what
/// is on screen still matches the world, and a mirror that quietly stopped
/// updating is indistinguishable from a world where nothing is moving.
fn draw_remote_status(
    ui: &mut egui::Ui,
    remote: ConnectionState,
    stale: Option<&str>,
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
            egui::Button::new(icons::ARROWS_CLOCKWISE),
        )
        .on_hover_text(
            "Rebuild & Relaunch — recompiles the project and reconnects. \
             Needed to pick up code added since it started, and the way \
             back from a project that exited.",
        )
        .clicked()
    {
        actions.push(EditorAction::RebuildRemote);
    }
    ui.label(egui::RichText::new(format!("{icon} {text}")).color(color))
        .on_hover_text(hover);
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

/// The engines this machine has, which of them is in use, and a way to
/// get rid of the rest.
///
/// # Why this is visible at all
///
/// A project builds against `~/.local/share/kooch/<version>/engine` and
/// nothing in the editor ever said so. The only way to find out which
/// engine a project was compiling against was to read a log line at the
/// moment it was replaced, or to look at the directory's timestamp — and
/// "which engine is this" is the first question when a build behaves
/// differently than it did yesterday.
///
/// ⚠️ New versions are not created from here. The version is the engine's
/// own `major.minor.patch`, so a new one appears when the editor that
/// ships it does; this lists what arrived and lets the old ones go.
fn draw_installed_engines(
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

            // 🔴 Neither of those two may be removed. The editor's is
            // what the next project to open is pointed at, and the
            // project's is what it builds against — deleting either
            // leaves a manifest naming a directory that is not there.
            if !is_editors && !in_use && ui.button("Remove").clicked() {
                actions.push(EditorAction::RemoveEngine(engine.version.clone()));
            }
        });
        ui.weak(engine.path.display().to_string());
        ui.add_space(4.0);
    }
}

/// The DLSS SDK: whether this machine has it, and the one button that
/// fetches it.
///
/// 🔴 The tick box is not a formality. NVIDIA's licence is accepted **by
/// use**, so the moment the editor puts the SDK on disk somebody has
/// accepted it — and it has to be the person here, having been shown
/// where the terms are. A download button that worked on the first click
/// would be accepting a licence on their behalf.
///
/// ⚠️ It installs the SDK; it does not enable DLSS. Nothing in this
/// engine calls it yet.
fn draw_dlss_sdk(ui: &mut egui::Ui, install: &mut crate::dlss_sdk::SdkInstall) {
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
///
/// Play spawns `cargo run` and the child inherits this process's
/// environment, so until this field existed the only way to hand a game
/// a `KOOCH_*` variable was to relaunch the editor with it set. Every
/// measurement this engine can make is one of those variables.
///
/// Applied on Apply rather than on every keystroke: a half-typed
/// `KOOCH_SHADING_PA` saved to disk is a line somebody later reads as
/// the setting they made.
fn draw_launch_env(
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
///
/// 🔴 Shown on every launch while something is missing, and never
/// otherwise. The check runs once at startup because installing any of
/// it ends in a reboot, so the answer cannot change underneath.
///
/// It does **not** install. On an image-based distribution — which both
/// of this project's Linux targets are — a package writes a new image and
/// needs a reboot, which is not something an editor may do behind a
/// dialog. What it removes is the web search: the requirement, why it is
/// needed, and the exact command for *this* machine.
pub(crate) fn draw_preflight_window(ctx: &egui::Context, report: &crate::preflight::Report) {
    if report.is_ready() {
        return;
    }
    let id = egui::Id::new("kooch_preflight_window_open");
    let mut open = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(true);
    if !open {
        return;
    }

    egui::Window::new("This machine cannot build a project yet")
        .open(&mut open)
        .resizable(true)
        .default_width(520.0)
        .collapsible(false)
        .show(ctx, |ui| {
            ui.label(
                "A project made with this editor compiles the engine, so it needs a \
                 Rust toolchain and the system libraries the engine links against. \
                 These are missing:",
            );
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

            match report.command() {
                Some(command) => {
                    ui.separator();
                    ui.add_space(4.0);
                    ui.label("Paste this whole block, then reopen the editor:");
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

/// Where the Settings window's open flag lives.
fn settings_open_id(ctx: &egui::Context) -> egui::Id {
    let _ = ctx;
    egui::Id::new("kooch_settings_window_open")
}

/// The Settings window: editor preferences, and the two things about
/// the open project that belong beside them.
///
/// A window rather than a menu, because a menu closes on any click it
/// does not consume and a text field inside one cannot be typed into.
/// It is also movable and resizable, which a path worth pasting needs.
///
/// ⚠️ It used to say "not per-project", and that stopped being true
/// before the launch environment arrived: the engine list below already
/// takes the open project's version and offers to move it.
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
