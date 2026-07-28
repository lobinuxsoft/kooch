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

use std::sync::Mutex;

use ome_core::LogBuffer;
use tracing::Level;

use super::{ConsoleState, render::draw_console};
use crate::panels::id_stability_probe::{drawing, install_logger};

/// Serialises against the other id-stability tests: the log is global.
static LOCK: Mutex<()> = Mutex::new(());

/// Lines shaped like the ones the editor actually logs — some with
/// structured fields, some without, because that difference is the bug.
fn fill(buffer: &LogBuffer, from: u32, count: u32) {
    for n in from..from + count {
        match n % 3 {
            0 => buffer.push_project(Level::INFO, "ome_physics", format!("a body spawned n={n}")),
            1 => buffer.push_project(
                Level::WARN,
                "ome_remote",
                format!("a joint is waiting entity={n} field=body_a target=none"),
            ),
            _ => buffer.push_project(Level::INFO, "ome_world", "streaming settled".to_owned()),
        }
    }
}

/// The reported case: the Console on screen while lines keep arriving.
#[test]
fn console_rows_keep_their_ids_as_lines_arrive() {
    install_logger();
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let buffer = LogBuffer::new();
    fill(&buffer, 0, 40);
    let mut state = ConsoleState::default();

    let complaints = drawing(6, |ui, frame| {
        // A line lands between frames, which is the ordinary state of a
        // console attached to a running project.
        fill(&buffer, 100 + frame as u32, 1);
        draw_console(ui, Some(&buffer), &mut state);
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
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let buffer = LogBuffer::new();
    fill(&buffer, 0, 40);
    let mut state = ConsoleState::default();

    let complaints = drawing(4, |ui, _| draw_console(ui, Some(&buffer), &mut state));

    drop(guard);
    assert!(
        complaints.is_empty(),
        "the Console gave {} widget(s) a new id with an unchanging log:\n{}",
        complaints.len(),
        complaints.join("\n"),
    );
}
