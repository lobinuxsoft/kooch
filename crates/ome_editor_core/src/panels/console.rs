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
                    ui.colored_label(
                        level_colour(entry.level),
                        mono(format!("{:<5}", level_name(entry.level))),
                    );
                    ui.colored_label(
                        target_colour(entry.from_project),
                        mono(short_target(&entry.target)),
                    );
                    draw_message(ui, entry);
                });
            }
        });
}

/// Monospaced text, padded so the level column lines up.
fn mono(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text).monospace()
}

/// Draws the message, with the structured fields picked out.
///
/// A line is `a sensor was entered a=8 b=9`, and the part anyone scans for
/// is the numbers. Rendering it as one flat string makes the reader do
/// that separation by eye, every line.
///
/// A warning or an error tints the whole message: at that point the
/// severity *is* the message, and a level chip four columns to the left is
/// not where the eye lands.
fn draw_message(ui: &mut egui::Ui, entry: &LogEntry) {
    let tint = match entry.level {
        Level::ERROR | Level::WARN => Some(level_colour(entry.level)),
        _ => None,
    };

    for part in split_fields(&entry.message) {
        match part {
            Part::Text(text) => match tint {
                Some(colour) => ui.colored_label(colour, mono(text)),
                None => ui.label(mono(text)),
            },
            Part::Field { key, value } => {
                ui.label(mono(key).color(FIELD_KEY));
                ui.label(mono(value).color(FIELD_VALUE).strong())
            }
        };
    }
}

/// A run of the message: prose, or one `key=value`.
enum Part<'a> {
    Text(&'a str),
    Field { key: &'a str, value: &'a str },
}

/// Splits a message into prose and `key=value` runs.
///
/// Whitespace-separated tokens containing `=` are fields; everything else
/// is prose, kept contiguous so wrapping behaves. Deliberately simple: a
/// message that happens to contain an equals sign is coloured slightly
/// oddly, which is a better failure than a parser that swallows a line.
fn split_fields(message: &str) -> Vec<Part<'_>> {
    let mut parts = Vec::new();
    let mut prose_start = None::<usize>;

    for token in message.split_inclusive(' ') {
        let trimmed = token.trim_end();
        let offset = token.as_ptr() as usize - message.as_ptr() as usize;

        match trimmed.split_once('=') {
            Some((key, value)) if !key.is_empty() && !key.contains(' ') => {
                if let Some(start) = prose_start.take() {
                    parts.push(Part::Text(message[start..offset].trim_end()));
                }
                parts.push(Part::Field {
                    key: &trimmed[..key.len() + 1],
                    value,
                });
            }
            _ => {
                prose_start.get_or_insert(offset);
            }
        }
    }
    if let Some(start) = prose_start {
        parts.push(Part::Text(message[start..].trim_end()));
    }
    parts
}

/// A project's own module, distinguished from the editor's without a
/// prefix cluttering every line.
fn target_colour(from_project: bool) -> egui::Color32 {
    match from_project {
        true => egui::Color32::from_rgb(150, 190, 150),
        false => egui::Color32::from_rgb(130, 130, 130),
    }
}

/// The `key=` of a structured field, and its value.
const FIELD_KEY: egui::Color32 = egui::Color32::from_rgb(150, 130, 190);
const FIELD_VALUE: egui::Color32 = egui::Color32::from_rgb(215, 190, 130);

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
            from_project: false,
        }
    }

    fn from_project(level: Level, target: &str, message: &str) -> LogEntry {
        LogEntry {
            from_project: true,
            ..entry(level, target, message)
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
        assert!(state.shows(&from_project(Level::INFO, "ome_physics", "a sensor fired")));
        assert!(!state.shows(&entry(Level::INFO, "handlers", "scene loaded")));
    }

    /// The point of carrying the project's own level: asking for warnings
    /// has to hide a project's info, which sniffing a prefix could never
    /// do — every forwarded line used to arrive as an `info`.
    #[test]
    fn a_projects_line_is_filtered_by_its_own_level() {
        let state = ConsoleState {
            level: Level::WARN,
            ..Default::default()
        };
        assert!(state.shows(&from_project(
            Level::WARN,
            "ome_physics",
            "a joint is waiting"
        )));
        assert!(!state.shows(&from_project(
            Level::INFO,
            "ome_physics",
            "a sensor was entered"
        )));
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

    /// The numbers are what anyone scans a log line for, so they have to
    /// come out of the prose.
    #[test]
    fn fields_are_separated_from_prose() {
        let parts = split_fields("a sensor was entered a=8 b=9");
        let rendered: Vec<String> = parts
            .iter()
            .map(|p| match p {
                Part::Text(t) => format!("text:{t}"),
                Part::Field { key, value } => format!("field:{key}{value}"),
            })
            .collect();
        assert_eq!(
            rendered,
            vec!["text:a sensor was entered", "field:a=8", "field:b=9"],
        );
    }

    /// A message with nothing structured in it stays one run, or wrapping
    /// breaks at every space.
    #[test]
    fn prose_without_fields_is_one_part() {
        let parts = split_fields("scene loaded from disk");
        assert_eq!(parts.len(), 1);
        assert!(matches!(parts[0], Part::Text("scene loaded from disk")));
    }

    /// An equals sign inside prose must not eat the line — colouring it
    /// oddly is a better failure than losing it.
    #[test]
    fn an_equals_in_prose_does_not_swallow_anything() {
        let parts = split_fields("solved x=1 for the case");
        let text: String = parts
            .iter()
            .map(|p| match p {
                Part::Text(t) => (*t).to_owned(),
                Part::Field { key, value } => format!("{key}{value}"),
            })
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(text, "solved x=1 for the case");
    }

    #[test]
    fn an_empty_message_produces_nothing() {
        assert!(split_fields("").is_empty());
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
