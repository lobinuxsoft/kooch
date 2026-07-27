//! Drawing the Console's rows.

use ome_core::{LogBuffer, LogEntry};
use tracing::Level;

use super::ConsoleState;

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

    // Before the controls, so a filter typed this frame is applied this
    // frame rather than one behind the keystroke.
    state.sync(buffer);

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

    ui.horizontal(|ui| {
        ui.weak(format!(
            "{} of {} lines",
            state.visible().len(),
            state.entries().len(),
        ));
        if state.dropped() > 0 {
            // Said rather than implied: a panel showing two thousand lines
            // out of nine thousand looks like nothing else happened.
            ui.weak(format!("({} older dropped)", state.dropped()));
        }
    });
    ui.separator();

    // One line per row, and only the rows on screen are built. Wrapping
    // would make rows different heights, and a virtualised list needs to
    // know where row N starts without having laid out the N-1 before it —
    // so a long line scrolls sideways instead of folding. That is the
    // trade this panel makes to stay free with a full log.
    let row_height =
        ui.text_style_height(&egui::TextStyle::Monospace) + ui.spacing().item_spacing.y;

    egui::ScrollArea::both()
        .stick_to_bottom(state.follow)
        .auto_shrink([false, false])
        .show_rows(ui, row_height, state.visible().len(), |ui, rows| {
            ui.spacing_mut().item_spacing.y = 1.0;
            let entries = state.entries();
            for &index in &state.visible()[rows] {
                let Some(entry) = entries.get(index) else {
                    continue;
                };
                ui.horizontal(|ui| {
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
