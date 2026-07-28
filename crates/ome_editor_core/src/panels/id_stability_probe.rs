//! Asking egui whether a panel keeps its widget ids from frame to frame.
//!
//! egui already performs this check and complains — `Widget rect … changed
//! id between passes` — but only at runtime, in debug, into a log nobody
//! reads until there are three hundred of them (#641). This turns the
//! complaint into a test failure.
//!
//! # Why it matters
//!
//! egui addresses interaction state by id: what is focused, what is being
//! dragged, which text you had selected. A widget whose id changes has
//! none of that carried over — a drag ends itself, a text cursor jumps
//! home, a selection you were about to copy disappears.
//!
//! # What "between passes" means here
//!
//! egui compares the previous pass with the current one. With a single
//! pass per frame — the ordinary case — that is **frame against frame**.
//! So a panel that is stable when its data holds still can still fail once
//! the data moves, which is why the probe hands the caller the frame
//! number and lets it change the world in between.

use std::sync::{Mutex, OnceLock};

/// Warnings egui emitted, collected by the logger installed below.
static WARNINGS: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Collector;

impl log::Log for Collector {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata())
            && let Ok(mut warnings) = WARNINGS.lock()
        {
            warnings.push(record.args().to_string());
        }
    }

    fn flush(&self) {}
}

/// Installs the collector once, whichever test gets there first.
pub(crate) fn install_logger() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        let _ = log::set_boxed_logger(Box::new(Collector));
        log::set_max_level(log::LevelFilter::Warn);
    });
}

/// Draws `frames` frames of `draw` and returns egui's id complaints.
///
/// The closure receives the frame number so it can let the world move
/// between frames — arriving log lines, a changing snapshot — which is
/// when this class of bug shows up.
///
/// Callers must hold their own lock: the collected warnings are global,
/// so two probes running at once read each other's complaints.
pub(crate) fn drawing(frames: usize, mut draw: impl FnMut(&mut egui::Ui, usize)) -> Vec<String> {
    install_logger();
    if let Ok(mut warnings) = WARNINGS.lock() {
        warnings.clear();
    }

    let ctx = egui::Context::default();
    for frame in 0..frames {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 700.0),
            )),
            ..Default::default()
        };
        ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| draw(ui, frame));
        });
    }

    WARNINGS
        .lock()
        .map(|warnings| {
            warnings
                .iter()
                .filter(|w| w.contains("changed id between passes"))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}
