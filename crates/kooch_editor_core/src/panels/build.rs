//! The Build panel — the list, the button, and cargo's output (#758).
//!
//! Deliberately small. A `.buildpreset` is a reflected asset (#744), so
//! *editing* one is the Inspector's job and costs this panel nothing:
//! what is left is the three things the Inspector cannot do.
//!
//! The alternative was a bespoke form with a field per setting, and the
//! evidence that bespoke forms do not keep up is that `RenderSettings`
//! had none for as long as it existed.

use kooch_core::Guid;

use crate::actions::EditorAction;
use crate::build::{BuildPreset, BuildStatus};
use crate::icons;

/// What the panel needs to draw itself.
pub(crate) struct BuildPanel {
    /// Every `.buildpreset` in the project, as `(guid, name, preset)`.
    pub presets: Vec<(Guid, String, BuildPreset)>,
    /// The running build, if there is one.
    pub status: Option<BuildStatus>,
    /// The last lines cargo produced.
    pub log: Vec<String>,
    /// Whether a project is open at all.
    pub project: bool,
}

/// Content of the "Build" tab.
pub(crate) fn draw_build_content(
    ui: &mut egui::Ui,
    panel: &BuildPanel,
    selected: &mut Option<Guid>,
    inspected: &mut Option<Guid>,
    actions: &mut Vec<EditorAction>,
) {
    if !panel.project {
        ui.weak("Open a project to build it.");
        return;
    }
    if panel.presets.is_empty() {
        ui.weak("No build presets yet.");
        // Where to go, not just what is missing: a panel that says
        // "nothing here" and stops is a dead end.
        ui.weak(format!(
            "{} Right-click a folder under assets/ → New Build Preset.",
            icons::PACKAGE,
        ));
        return;
    }

    // A preset selected but since deleted would leave the button acting
    // on nothing.
    if !panel
        .presets
        .iter()
        .any(|(guid, ..)| Some(*guid) == *selected)
    {
        // The first one. There is no "the runnable preset" any more —
        // the list is the selector, and a field that only decided which
        // row got a different icon was not one.
        *selected = panel.presets.first().map(|(guid, ..)| *guid);
    }

    draw_presets(ui, panel, selected, inspected, actions);
    ui.separator();
    draw_status(ui, panel);
    ui.separator();
    draw_log(ui, &panel.log);
}

fn draw_presets(
    ui: &mut egui::Ui,
    panel: &BuildPanel,
    selected: &mut Option<Guid>,
    inspected: &mut Option<Guid>,
    actions: &mut Vec<EditorAction>,
) {
    for (guid, name, preset) in &panel.presets {
        let chosen = Some(*guid) == *selected;
        // The floor earns a place in the label because it is the
        // difference between a build that runs on the handheld and one
        // that stops at a missing symbol version — and nothing else in
        // the row hints at it.
        let floor = match preset.glibc_floor() {
            Some(floor) => format!(", glibc {floor}+"),
            None => String::new(),
        };
        // The mode leads the row: it is the difference between a build
        // you hand out and one that opens a listening socket, and it is
        // the field somebody is most likely to have left on the wrong
        // setting.
        let label = format!(
            "{} {name}  ({}, {}{floor})",
            match preset.is_profiling() {
                true => icons::CHART_BAR,
                false => icons::PACKAGE,
            },
            preset.mode_label(),
            match preset.is_host() {
                true => "this machine",
                false => preset.target_triple.trim(),
            },
        );
        if ui.selectable_label(chosen, label).clicked() {
            *selected = Some(*guid);
            // Shown in the Inspector, which is where it is edited — this
            // panel deliberately draws no fields of its own. Set
            // directly, the way the Asset Browser does it: the selection
            // is editor state, not an edit.
            *inspected = Some(*guid);
        }
    }

    ui.add_space(4.0);
    let building = matches!(
        panel.status,
        Some(BuildStatus::Compiling | BuildStatus::Packaging),
    );
    let button = ui.add_enabled(
        !building && selected.is_some(),
        egui::Button::new(format!("{} Build", icons::PACKAGE)),
    );
    if building {
        button.on_disabled_hover_text("A build is already running.");
    } else if button.clicked()
        && let Some(guid) = *selected
    {
        actions.push(EditorAction::BuildProject(guid));
    }

    // Only while there is something to stop. A permanently visible
    // Cancel that does nothing most of the time is a button people learn
    // to ignore.
    if building
        && ui
            .button(format!("{} Cancel", icons::TRASH))
            .on_hover_text("Stops cargo. Nothing is packaged, so no half-built game is written.")
            .clicked()
    {
        actions.push(EditorAction::CancelBuild);
    }
}

fn draw_status(ui: &mut egui::Ui, panel: &BuildPanel) {
    match &panel.status {
        None => ui.weak("Idle."),
        Some(BuildStatus::Compiling) => {
            ui.horizontal(|ui| {
                ui.spinner();
                // Named, because the first build of a project compiles
                // the engine and someone watching a still panel for four
                // minutes reasonably concludes it hung.
                ui.label("Compiling — the first build of a project takes minutes.");
            })
            .response
        }
        Some(BuildStatus::Packaging) => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Packaging assets…");
            })
            .response
        }
        Some(BuildStatus::Done(package)) => {
            let summary = format!(
                "{} Built: {} — {} assets, {} scenes",
                icons::PACKAGE,
                package.dir.display(),
                package.assets,
                package.scenes,
            );
            let response = ui.strong(summary);
            if !package.shadowed.is_empty() {
                // Worth surfacing: the engine's version of those files is
                // simply not in the build, and nothing else says so.
                ui.weak(format!(
                    "{} of your assets replaced an engine asset of the same name.",
                    package.shadowed.len(),
                ))
                .on_hover_text(package.shadowed.join("\n"));
            }
            response
        }
        // Not red: nothing went wrong, someone pressed a button, and an
        // error where a deliberate act happened reads as a bug.
        Some(BuildStatus::Cancelled) => ui.weak("Cancelled."),
        Some(BuildStatus::Failed(why)) => ui.colored_label(egui::Color32::LIGHT_RED, why),
    };
}

/// Cargo's own words, newest at the bottom.
///
/// Its output, not a summary of it: when a build fails the only useful
/// question is what the compiler said, and paraphrasing loses the line
/// number.
///
/// 🔴 Selectable, and with a button that copies the lot. An error nobody
/// can get out of the window is an error nobody can paste into a search,
/// a bug report or a message — which is most of what someone does with a
/// compiler error. egui labels are not selectable by default, so this was
/// a log you could read and not use.
fn draw_log(ui: &mut egui::Ui, log: &[String]) {
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                !log.is_empty(),
                egui::Button::new(format!("{} Copy log", icons::COPY)),
            )
            .clicked()
        {
            ui.ctx().copy_text(log.join("\n"));
        }
        ui.weak(format!("{} lines", log.len()));
    });
    egui::ScrollArea::vertical()
        .stick_to_bottom(true)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for line in log {
                ui.add(
                    egui::Label::new(egui::RichText::new(line).monospace())
                        .selectable(true)
                        // Compiler output is pre-wrapped and its columns
                        // line up: rewrapping it turns a caret under a
                        // token into a caret under nothing.
                        .wrap_mode(egui::TextWrapMode::Extend),
                );
            }
        });
}

/// Every `.buildpreset` the project holds, loaded.
///
/// Read here rather than kept in a resource: presets are edited in the
/// Inspector, which writes the file, and a cached copy would be the one
/// the Build button used — so a preset someone just changed would build
/// with its old settings.
pub(crate) fn presets_in(
    resources: &mut kooch_core::resource::Resources,
    catalog: &[crate::panels::inspector::AssetCatalogEntry],
) -> Vec<(Guid, String, BuildPreset)> {
    let wanted = std::any::type_name::<BuildPreset>();
    let mut found: Vec<(Guid, String, BuildPreset)> = catalog
        .iter()
        .filter(|entry| entry.type_name == wanted)
        .filter_map(|entry| {
            let handle = kooch_ecs::reflect::asset_registry::load_handle::<BuildPreset>(
                resources, entry.guid,
            )?;
            let preset = resources
                .get::<kooch_core::assets::Assets<BuildPreset>>()?
                .get(handle)?
                .clone();
            Some((entry.guid, entry.display_name.clone(), preset))
        })
        .collect();
    // By name, so the list does not reshuffle as the catalog is rebuilt.
    found.sort_by(|a, b| a.1.cmp(&b.1));
    found
}
