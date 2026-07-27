//! The Console tab — what the engine has been saying.
//!
//! Everything reaches this through `tracing`, including a running
//! project's output, which the editor forwards as `[game] ...`. So there is
//! one log rather than a panel and a terminal that drift apart.
//!
//! # Why filters are not optional
//!
//! At `info` the engine is chatty — asset scans, meshlet LOD chains, one
//! line per pipeline. The line that matters is a `warn` about a joint with
//! no bodies, three hundred lines up. A log nobody can narrow is a log
//! nobody reads, which is the state this panel replaces.

use ome_core::{LogBuffer, LogEntry};
use tracing::Level;

/// What the panel is currently showing, kept between frames.
pub(crate) struct ConsoleState {
    /// Minimum level shown.
    pub(crate) level: Level,
    /// Substring filter, matched against the message and the target.
    pub(crate) filter: String,
    /// Follow the tail. Turned off by scrolling up, so reading something
    /// is not fought by new lines arriving.
    pub(crate) follow: bool,
    /// Show only what a spawned project said.
    pub(crate) project_only: bool,
}

impl Default for ConsoleState {
    fn default() -> Self {
        Self {
            // Not `INFO`: the interesting lines are warnings and errors,
            // and the first thing anyone does at info is scroll past a
            // hundred asset lines. The level picker is right there.
            level: Level::INFO,
            filter: String::new(),
            follow: true,
            project_only: false,
        }
    }
}

impl ConsoleState {
    /// Whether an entry passes the current filters.
    pub(crate) fn shows(&self, entry: &LogEntry) -> bool {
        // `tracing::Level` orders with ERROR as the *smallest*, so "at
        // least this severe" is `<=`. Getting this backwards shows debug
        // spam when someone asks for errors, which is the wrong way to be
        // wrong.
        if entry.level > self.level {
            return false;
        }
        if self.project_only && !entry.is_from_project() {
            return false;
        }
        if self.filter.is_empty() {
            return true;
        }
        let needle = self.filter.to_lowercase();
        entry.message.to_lowercase().contains(&needle)
            || entry.target.to_lowercase().contains(&needle)
    }
}

/// Content of the "Console" tab.
pub(crate) fn draw_console(
    ui: &mut egui::Ui,
    buffer: Option<&LogBuffer>,
    state: &mut ConsoleState,
) {
    let Some(buffer) = buffer else {
        ui.weak("No log buffer — this host did not install one.");
        return;
    };

    let entries = buffer.snapshot();
    let dropped = buffer.dropped();

    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("console_level")
            .selected_text(level_name(state.level))
            .width(90.0)
            .show_ui(ui, |ui| {
                for level in [
                    Level::ERROR,
                    Level::WARN,
                    Level::INFO,
                    Level::DEBUG,
                    Level::TRACE,
                ] {
                    ui.selectable_value(&mut state.level, level, level_name(level));
                }
            });

        ui.add(
            egui::TextEdit::singleline(&mut state.filter)
                .hint_text("Filter")
                .desired_width(180.0),
        );

        ui.checkbox(&mut state.project_only, "Project only")
            .on_hover_text("Only lines the running project produced");
        ui.checkbox(&mut state.follow, "Follow")
            .on_hover_text("Scroll to the newest line as it arrives");

        if ui.button("Clear").clicked() {
            buffer.clear();
        }
    });

    let shown = entries.iter().filter(|e| state.shows(e)).count();
    ui.horizontal(|ui| {
        ui.weak(format!("{shown} of {} lines", entries.len()));
        if dropped > 0 {
            // Said rather than implied: a panel showing two thousand lines
            // out of nine thousand looks like nothing else happened.
            ui.weak(format!("({dropped} older dropped)"));
        }
    });
    ui.separator();

    egui::ScrollArea::vertical()
        .stick_to_bottom(state.follow)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            for entry in entries.iter().filter(|e| state.shows(e)) {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.colored_label(level_colour(entry.level), level_name(entry.level));
                    ui.weak(short_target(&entry.target));
                    ui.label(&entry.message);
                });
            }
        });
}

fn level_name(level: Level) -> &'static str {
    match level {
        Level::ERROR => "ERROR",
        Level::WARN => "WARN",
        Level::INFO => "INFO",
        Level::DEBUG => "DEBUG",
        _ => "TRACE",
    }
}

/// Red and amber for the two that matter; everything else recedes.
fn level_colour(level: Level) -> egui::Color32 {
    match level {
        Level::ERROR => egui::Color32::from_rgb(235, 90, 80),
        Level::WARN => egui::Color32::from_rgb(240, 180, 40),
        Level::INFO => egui::Color32::from_rgb(140, 170, 210),
        _ => egui::Color32::GRAY,
    }
}

/// The last segment of a module path.
///
/// `ome_editor_core::actions::handlers` is thirty characters of mostly
/// nothing on every line; `handlers` is the part that differs.
fn short_target(target: &str) -> &str {
    target.rsplit("::").next().unwrap_or(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(level: Level, target: &str, message: &str) -> LogEntry {
        LogEntry {
            seq: 0,
            level,
            target: target.to_owned(),
            message: message.to_owned(),
        }
    }

    /// `tracing::Level` orders ERROR smallest, so "at least this severe"
    /// is `<=`. Backwards means asking for errors shows trace spam.
    #[test]
    fn the_level_filter_keeps_the_more_severe() {
        let state = ConsoleState {
            level: Level::WARN,
            ..Default::default()
        };
        assert!(state.shows(&entry(Level::ERROR, "t", "louder")));
        assert!(state.shows(&entry(Level::WARN, "t", "equal")));
        assert!(!state.shows(&entry(Level::INFO, "t", "quieter")));
        assert!(!state.shows(&entry(Level::DEBUG, "t", "quieter still")));
    }

    #[test]
    fn the_text_filter_matches_message_or_target() {
        let state = ConsoleState {
            filter: "joint".to_owned(),
            ..Default::default()
        };
        assert!(state.shows(&entry(Level::INFO, "t", "a joint broke")));
        assert!(state.shows(&entry(Level::INFO, "ome_physics::joint", "unrelated")));
        assert!(!state.shows(&entry(Level::INFO, "t", "a collider")));
    }

    /// Someone typing "JOINT" means the same thing.
    #[test]
    fn the_text_filter_ignores_case() {
        let state = ConsoleState {
            filter: "JOINT".to_owned(),
            ..Default::default()
        };
        assert!(state.shows(&entry(Level::INFO, "t", "a joint broke")));
    }

    /// The question this panel exists for — "did my trigger fire" — is
    /// asked about the project, not the editor.
    #[test]
    fn project_only_hides_the_editors_own_lines() {
        let state = ConsoleState {
            project_only: true,
            ..Default::default()
        };
        assert!(state.shows(&entry(Level::INFO, "t", "[game] a sensor fired")));
        assert!(!state.shows(&entry(Level::INFO, "t", "scene loaded")));
    }

    /// The filters compose; passing one is not enough.
    #[test]
    fn the_filters_are_combined() {
        let state = ConsoleState {
            level: Level::WARN,
            filter: "joint".to_owned(),
            ..Default::default()
        };
        assert!(state.shows(&entry(Level::WARN, "t", "a joint broke")));
        assert!(
            !state.shows(&entry(Level::INFO, "t", "a joint broke")),
            "the level filter was ignored once the text matched",
        );
        assert!(
            !state.shows(&entry(Level::WARN, "t", "a collider")),
            "the text filter was ignored once the level matched",
        );
    }

    #[test]
    fn a_long_module_path_is_shortened_to_its_last_segment() {
        assert_eq!(
            short_target("ome_editor_core::actions::handlers"),
            "handlers"
        );
        assert_eq!(short_target("physics_smoke"), "physics_smoke");
    }
}
