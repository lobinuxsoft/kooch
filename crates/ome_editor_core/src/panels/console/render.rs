//! Drawing the Console's rows.

use ome_core::{LogBuffer, LogEntry};
use tracing::Level;

use super::{ALL_LEVELS, ConsoleState};

/// Content of the "Console" tab.
pub(crate) fn draw_console(
    ui: &mut egui::Ui,
    focused: bool,
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
        // One toggle per severity rather than a "minimum level" dropdown.
        // A threshold cannot express "hide the warnings, keep the rest",
        // which is the thing anyone wants when one warning repeats three
        // hundred times and buries the log (#641).
        for level in ALL_LEVELS {
            let mut shown = state.levels.shows(level);
            let label = egui::RichText::new(level_name(level)).color(match shown {
                true => level_colour(level),
                false => egui::Color32::DARK_GRAY,
            });
            if ui
                .toggle_value(&mut shown, label)
                .on_hover_text(format!("Show {} lines", level_name(level)))
                .changed()
            {
                state.levels.set(level, shown);
            }
        }
        if state.levels.is_empty() {
            // An empty Console otherwise reads as "nothing happened".
            ui.colored_label(
                egui::Color32::from_rgb(240, 180, 40),
                "\u{26a0} every level is off",
            );
        }
        ui.separator();

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

        // Copies what the filter currently shows, not the whole buffer:
        // the lines on screen are the ones being looked at, and a
        // thousand-line dump is not what anyone pastes into a report.
        let visible = state.visible().len();
        if ui
            .add_enabled(visible > 0, egui::Button::new("Copy"))
            .on_hover_text("Copy the lines this filter shows")
            .clicked()
        {
            let text: String = state
                .visible()
                .iter()
                .filter_map(|&i| state.entries().get(i))
                .map(line_as_text)
                .collect::<Vec<_>>()
                .join("\n");
            ui.ctx().copy_text(text);
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

    if focused {
        handle_keyboard(ui, state);
    }

    // One line per row, and only the rows on screen are built. Wrapping
    // would make rows different heights, and a virtualised list needs to
    // know where row N starts without having laid out the N-1 before it —
    // so a long line scrolls sideways instead of folding. That is the
    // trade this panel makes to stay free with a full log.
    let row_height =
        ui.text_style_height(&egui::TextStyle::Monospace) + ui.spacing().item_spacing.y;

    let mut area = egui::ScrollArea::both()
        .id_salt("console_rows")
        .stick_to_bottom(state.follow)
        .auto_shrink([false, false]);
    // Offset rather than `scroll_to_me`: the cursor's row is often not
    // built, which is the whole reason a moved cursor looked like nothing.
    if state.take_scroll_request()
        && let Some(row) = state.cursor()
    {
        let centred = row as f32 * row_height - ui.available_height() * 0.5;
        area = area.vertical_scroll_offset(centred.max(0.0));
    }
    area.show_rows(ui, row_height, state.visible().len(), |ui, rows| {
        ui.spacing_mut().item_spacing.y = 1.0;
        let entries = state.entries();
        let dropped = state.dropped();
        let cursor = state.cursor();
        for (offset, &index) in state.visible()[rows.clone()].iter().enumerate() {
            let is_cursor = cursor == Some(rows.start + offset);
            let Some(entry) = entries.get(index) else {
                continue;
            };
            // Keyed on the line, not on the slot it landed in.
            //
            // Without this every widget takes an automatic id, which
            // egui hands out by order of creation — so a row emitting a
            // different number of fragments renames every widget after
            // it. Rows are a fixed height, so nothing moves on screen:
            // same rect, new id, which is exactly what egui reports
            // (#641). And the report is itself a log line, which shifts
            // the rows again — that is why one bad frame produced three
            // hundred of them.
            //
            // The absolute sequence, not the index: the buffer drops
            // from the front, so index 0 is a different line after
            // every eviction.
            let seq = dropped + index as u64;
            ui.push_id(seq, |ui| {
                let row = ui.horizontal(|ui| {
                    // Painted behind the text: moving a cursor nobody
                    // can see is the same as not moving it.
                    if is_cursor {
                        let mut band = ui.available_rect_before_wrap();
                        band.set_height(ui.text_style_height(&egui::TextStyle::Monospace));
                        ui.painter().rect_filled(
                            band,
                            0.0,
                            ui.visuals().selection.bg_fill.linear_multiply(0.45),
                        );
                    }
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

                // Right-click the row for the one line, when the Copy
                // button's "everything shown" is more than wanted.
                row.response.context_menu(|ui| {
                    if ui.button("Copy line").clicked() {
                        ui.ctx().copy_text(line_as_text(entry));
                        ui.close();
                    }
                });
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
        // `selectable(true)` rather than `ui.label`: a plain label cannot
        // be dragged over, so the text was unreachable — a log you can
        // read and not quote is half a log. Selection still stops at each
        // fragment, which is what the copy actions below are for.
        match part {
            Part::Text(text) => {
                let rich = match tint {
                    Some(colour) => mono(text).color(colour),
                    None => mono(text),
                };
                ui.add(egui::Label::new(rich).selectable(true))
            }
            Part::Field { key, value } => {
                ui.add(egui::Label::new(mono(key).color(FIELD_KEY)).selectable(true));
                ui.add(egui::Label::new(mono(value).color(FIELD_VALUE).strong()).selectable(true))
            }
        };
    }
}

/// One log line as plain text, the way someone would paste it.
///
/// Rebuilt from the entry rather than from the drawn fragments: what is
/// on screen is split into coloured pieces for scanning, and pasting that
/// separation into a bug report helps nobody.
pub(super) fn line_as_text(entry: &LogEntry) -> String {
    format!(
        "{} {} {}",
        level_name(entry.level),
        entry.target,
        entry.message
    )
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

/// Moves the console's cursor with the keyboard.
///
/// Only reached when this panel has focus; the arrows belong to whichever
/// panel the user last clicked (#661).
fn handle_keyboard(ui: &egui::Ui, state: &mut ConsoleState) {
    // A text field with keyboard focus owns the arrows — moving a caret is
    // what they mean there. The filter box is one click away from every
    // row in this panel, so without this the panel and the field fight
    // over every keystroke and the field wins silently.
    if ui.memory(|m| m.focused().is_some()) {
        return;
    }

    // A page is a screenful of rows rather than a fixed number, so the key
    // means the same thing in a tall panel and a short one.
    let row_height = ui.text_style_height(&egui::TextStyle::Monospace) + 1.0;
    let page = ((ui.available_height() / row_height).floor() as isize - 1).max(1);

    let (up, down, page_up, page_down, home, end, copy) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::ArrowUp),
            i.key_pressed(egui::Key::ArrowDown),
            i.key_pressed(egui::Key::PageUp),
            i.key_pressed(egui::Key::PageDown),
            i.key_pressed(egui::Key::Home),
            i.key_pressed(egui::Key::End),
            i.modifiers.command && i.key_pressed(egui::Key::C),
        )
    });

    match (up, down, page_up, page_down) {
        (true, _, _, _) => state.move_cursor(-1),
        (_, true, _, _) => state.move_cursor(1),
        (_, _, true, _) => state.move_cursor(-page),
        (_, _, _, true) => state.move_cursor(page),
        _ => {}
    }
    if home {
        state.cursor_to_edge(false);
    }
    if end {
        // End means "back to the newest", so it also resumes following:
        // asking for the bottom of a log is asking to keep seeing it.
        state.cursor_to_edge(true);
        state.follow = true;
    }

    // Ctrl+C copies the highlighted line. The panel's Copy button takes
    // everything the filter shows; this takes the one line being read.
    if copy && let Some(entry) = state.cursor_line() {
        ui.ctx().copy_text(line_as_text(entry));
    }
}
