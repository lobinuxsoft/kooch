//! The Console's rows have to keep their ids as lines arrive.
//!
//! A row's widgets took automatic ids, which egui hands out by order of
//! creation. [`draw_message`](super::render) emits a *variable* number of
//! them — one per text run plus two per `key=value` pair — so a row whose
//! content changes shifts the ids of every row after it.
//!
//! Rows are a fixed height, so nothing moves on screen. Same rect, new id,
//! which is precisely what egui complains about (#641). And because each
//! complaint is itself logged, it becomes another line, which shifts the
//! rows again: the warning feeds itself. That is why one bad frame
//! produced three hundred of them.

use kooch_core::LogBuffer;
use tracing::Level;

use super::{ConsoleState, render::draw_console};
use crate::panels::id_stability_probe::{drawing, install_logger};

/// Lines shaped like the ones the editor actually logs — some with
/// structured fields, some without, because that difference is the bug.
fn fill(buffer: &LogBuffer, from: u32, count: u32) {
    for n in from..from + count {
        match n % 3 {
            0 => buffer.push_project(Level::INFO, "kooch_physics", format!("a body spawned n={n}")),
            1 => buffer.push_project(
                Level::WARN,
                "kooch_remote",
                format!("a joint is waiting entity={n} field=body_a target=none"),
            ),
            _ => buffer.push_project(Level::INFO, "kooch_world", "streaming settled".to_owned()),
        }
    }
}

/// The reported case: the Console on screen while lines keep arriving.
#[test]
fn console_rows_keep_their_ids_as_lines_arrive() {
    install_logger();
    let guard = crate::panels::id_stability_probe::PROBE_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let buffer = LogBuffer::new();
    // Far more lines than fit, so `show_rows` actually virtualises and the
    // visible window moves as the tail arrives. With a log that fits on
    // screen nothing scrolls, and nothing scrolling is not the reported
    // case.
    fill(&buffer, 0, 400);
    let mut state = ConsoleState::default();

    let complaints = drawing(6, |ui, frame| {
        // Several lines land between frames — the state of a console
        // attached to a project that is talking.
        fill(&buffer, 1000 + frame as u32 * 10, 7);
        draw_console(ui, false, Some(&buffer), &mut state);
    });

    drop(guard);
    assert!(
        complaints.is_empty(),
        "the Console gave {} widget(s) a new id as lines arrived:\n{}",
        complaints.len(),
        complaints.join("\n"),
    );
}

/// With no new lines at all, nothing may move either — this separates
/// "arrival shifts the ids" from "the rows were never stable".
#[test]
fn console_rows_are_stable_when_nothing_arrives() {
    install_logger();
    let guard = crate::panels::id_stability_probe::PROBE_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let buffer = LogBuffer::new();
    fill(&buffer, 0, 400);
    let mut state = ConsoleState::default();

    let complaints = drawing(4, |ui, _| {
        draw_console(ui, false, Some(&buffer), &mut state)
    });

    drop(guard);
    assert!(
        complaints.is_empty(),
        "the Console gave {} widget(s) a new id with an unchanging log:\n{}",
        complaints.len(),
        complaints.join("\n"),
    );
}

/// The reported gesture, at last: the mouse **over** the panel while the
/// log scrolls, with the clock running.
///
/// Three things the earlier tests left out, each of which alone makes the
/// bug impossible:
///
/// 1. **A pointer.** A `ScrollArea`'s bar widens on hover, animated.
/// 2. **A wheel.** Rows leave the visible window at the top and bottom,
///    which is where the user says it happens.
/// 3. **A clock.** egui's animations run on `stable_dt`; with time frozen
///    the bar never animates and the layout never wobbles.
#[test]
fn scrolling_under_the_mouse_keeps_the_row_ids() {
    use crate::panels::id_stability_probe::{Frame, drawing_with};

    install_logger();
    let guard = crate::panels::id_stability_probe::PROBE_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let buffer = LogBuffer::new();
    fill(&buffer, 0, 400);
    let mut state = ConsoleState::default();
    // Following the tail would pin the view to the bottom; the reported
    // case is reading *back* through the log.
    state.follow = false;

    let complaints = drawing_with(30, |ui, frame| {
        draw_console(ui, false, Some(&buffer), &mut state);
        Frame {
            // Over the rows, near the right edge where the bar lives.
            pointer: Some(egui::pos2(880.0, 300.0 + (frame % 5) as f32 * 20.0)),
            // Back and forth, so rows keep entering and leaving at both
            // edges rather than settling at one end.
            scroll: egui::vec2(0.0, if frame % 10 < 5 { -90.0 } else { 90.0 }),
        }
    });

    drop(guard);
    assert!(
        complaints.is_empty(),
        "scrolling under the mouse renamed {} widget(s):\n{}",
        complaints.len(),
        complaints
            .iter()
            .take(8)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n"),
    );
}

/// The same gesture, but inside a dock — which is where the editor draws.
///
/// Every test above puts the panel in a bare `CentralPanel`. The editor
/// puts it in `egui_dock`, and the reported log carries rects that only a
/// dock produces: a 28x28 tab and a 250x646 panel frame. That is the one
/// structural difference between the reproductions that pass and the
/// editor that does not.
#[test]
fn scrolling_inside_a_dock_keeps_the_row_ids() {
    use crate::panels::id_stability_probe::{Frame, drawing_with};
    use egui_dock::{DockArea, DockState};

    struct Viewer<'a> {
        buffer: &'a LogBuffer,
        state: &'a mut ConsoleState,
    }

    impl egui_dock::TabViewer for Viewer<'_> {
        type Tab = String;

        fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
            tab.as_str().into()
        }

        fn ui(&mut self, ui: &mut egui::Ui, _tab: &mut Self::Tab) {
            draw_console(ui, false, Some(self.buffer), self.state);
        }
    }

    install_logger();
    let guard = crate::panels::id_stability_probe::PROBE_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let buffer = LogBuffer::new();
    fill(&buffer, 0, 400);
    let mut state = ConsoleState::default();
    state.follow = false;
    let mut dock = DockState::new(vec!["Console".to_owned()]);

    let complaints = drawing_with(30, |ui, frame| {
        DockArea::new(&mut dock).show_inside(
            ui,
            &mut Viewer {
                buffer: &buffer,
                state: &mut state,
            },
        );
        Frame {
            pointer: Some(egui::pos2(880.0, 300.0 + (frame % 5) as f32 * 20.0)),
            scroll: egui::vec2(0.0, if frame % 10 < 5 { -90.0 } else { 90.0 }),
        }
    });

    drop(guard);
    assert!(
        complaints.is_empty(),
        "scrolling in a docked Console renamed {} widget(s):\n{}",
        complaints.len(),
        complaints
            .iter()
            .take(8)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n"),
    );
}
