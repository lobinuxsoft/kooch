//! Asking egui whether a panel keeps its widget ids from frame to frame.

use std::sync::{Mutex, OnceLock};

/// Warnings egui emitted, collected by the logger installed below.
static WARNINGS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Held for the whole of one probe run, by every caller.
pub(crate) static PROBE_LOCK: Mutex<()> = Mutex::new(());

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

/// What the probe feeds egui for one frame, beyond the drawing itself.
#[derive(Default)]
pub(crate) struct Frame {
    /// Where the mouse is. `None` means the pointer is off-screen.
    pub pointer: Option<egui::Pos2>,
    /// Scroll wheel delta, in points.
    pub scroll: egui::Vec2,
}

/// Draws `frames` frames of `draw` and returns egui's id complaints.
pub(crate) fn drawing_with(
    frames: usize,
    mut draw: impl FnMut(&mut egui::Ui, usize) -> Frame,
) -> Vec<String> {
    install_logger();
    if let Ok(mut warnings) = WARNINGS.lock() {
        warnings.clear();
    }

    let ctx = egui::Context::default();
    let mut next = Frame::default();
    for frame in 0..frames {
        // 60 fps. Real enough for animations to progress a little each
        // frame, which is when a layout that depends on them wobbles.
        let dt = 1.0 / 60.0;
        let mut events = Vec::new();
        if next.scroll != egui::Vec2::ZERO {
            events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: next.scroll,
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            });
        }
        if let Some(pos) = next.pointer {
            events.push(egui::Event::PointerMoved(pos));
        }

        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 700.0),
            )),
            time: Some(frame as f64 * dt),
            predicted_dt: dt as f32,
            events,
            ..Default::default()
        };

        let mut produced = Frame::default();
        // `run_ui` gives the root `Ui` the editor's own render loop uses,
        // so the probe draws through the same path egui 0.35 expects.
        ctx.run_ui(input, |ui| {
            egui::CentralPanel::default().show(ui, |ui| produced = draw(ui, frame));
        });
        next = produced;
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

/// [`drawing_with`], for callers with no pointer or wheel to simulate.
pub(crate) fn drawing(frames: usize, mut draw: impl FnMut(&mut egui::Ui, usize)) -> Vec<String> {
    drawing_with(frames, |ui, frame| {
        draw(ui, frame);
        Frame::default()
    })
}
