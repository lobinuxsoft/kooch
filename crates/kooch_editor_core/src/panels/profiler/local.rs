//! The profiler reading **this process** — the editor's own frame.

use egui::Ui;

/// Our own `GlobalProfilerUi`, kept alive for the whole session.
static PROFILER_UI: std::sync::OnceLock<std::sync::Mutex<puffin_egui::GlobalProfilerUi>> =
    std::sync::OnceLock::new();

/// Draws the flamegraph for this process.
pub(super) fn draw(ui: &mut Ui) {
    // 🔴 Nothing is switched on here. Recording starts only when the Record button is pressed, and
    // puffin's own default for `are_scopes_on` is already `false`, so an editor that never opens
    // this panel — or opens it and leaves it stopped — pays about 1 ns per scope and nothing else.
    let mut profiler_ui = PROFILER_UI
        .get_or_init(Default::default)
        .lock()
        .expect("the profiler ui mutex is never held across a panic");

    capture_controls(ui, &profiler_ui);
    ui.separator();

    // 🔴 The panel is not part of the frame it is reporting on.
    let was_on = puffin::are_scopes_on();
    puffin::set_scopes_on(false);
    if was_on {
        // 🔴 No flamegraph while recording, and this is the fix for the observer costing more than
        // the thing observed.
        recording_summary(ui, profiler_ui.global_frame_view());
    } else {
        profiler_ui.ui(ui);
    }
    puffin::set_scopes_on(was_on);
}

/// Clearing the history, and taking a capture off the machine.
fn capture_controls(ui: &mut Ui, profiler_ui: &puffin_egui::GlobalProfilerUi) {
    let frame_view = profiler_ui.global_frame_view();
    let recording = puffin::are_scopes_on();

    ui.horizontal(|ui| {
        // 🔴 This is NOT `puffin_egui`'s ▶/⏸ below, and the difference is the whole reason it
        // exists. That one freezes the VIEW on a frame while the scopes keep being recorded behind
        // it.
        let (label, hover) = if recording {
            (
                "⏹ Stop",
                "Stops recording. Scopes cost an atomic load and nothing else.",
            )
        } else {
            (
                "⏺ Record",
                "Starts recording. Costs 50-200 ns per scope while it runs.",
            )
        };
        if ui.button(label).on_hover_text(hover).clicked() {
            let starting = !recording;
            puffin::set_scopes_on(starting);
            if starting {
                // Ask again in a couple of frames' time — see
                // `SNAPSHOT_COUNTDOWN`.
                super::SNAPSHOT_COUNTDOWN.store(120, std::sync::atomic::Ordering::Relaxed);
                // 🔴 Every time recording starts, not only after a clear. A `FrameView` resolves the
                // scope ids inside a frame through a collection it builds as frames arrive, and
                // anything that replaces the view — Clear, Load — starts that collection empty.
                puffin::GlobalProfiler::lock().emit_scope_snapshot();
            }
        }

        ui.separator();
        if ui
            .button(format!("{} Clear history", crate::icons::TRASH))
            .on_hover_text("Drops every recorded frame. Recording continues.")
            .clicked()
        {
            // 🔴 Replaced rather than cleared, because puffin has no "clear everything":
            // `FrameView::clear_slowest` drops only the frames it kept for being slow, which is the
            // opposite of what a measurement needs — the slow frames are the interesting ones.
            *frame_view.lock() = puffin::FrameView::default();

            // 🔴 And a fresh view has no scope COLLECTION, which is what turns the scope ids inside
            // a frame back into names. Frames kept arriving after a clear and the panel drew
            // nothing at all, because there was no way left to say what any of them were.
            puffin::GlobalProfiler::lock().emit_scope_snapshot();
        }

        if ui
            // No icon: the icon table's own header records that eleven
            // of its first thirty codepoints drew something other than
            // what they were named, and there is no save glyph in it yet.
            .button("Save capture")
            .on_hover_text("Writes a .puffin file the standalone viewer can open")
            .clicked()
        {
            match save_capture(&frame_view) {
                Ok(path) => tracing::info!(
                    target: "kooch_editor::profiler",
                    path = %path.display(),
                    "wrote a profiler capture",
                ),
                Err(error) => tracing::error!(
                    target: "kooch_editor::profiler",
                    %error,
                    "could not write the profiler capture",
                ),
            }
        }

        if ui
            .button("Load capture")
            .on_hover_text("Opens the newest .puffin from the captures folder, right here")
            .clicked()
        {
            match load_latest_capture() {
                Ok(Some(view)) => {
                    *frame_view.lock() = view;
                    puffin::set_scopes_on(false);
                }
                Ok(None) => tracing::warn!(
                    target: "kooch_editor::profiler",
                    "no capture to load",
                ),
                Err(error) => tracing::error!(
                    target: "kooch_editor::profiler",
                    %error,
                    "could not read the capture",
                ),
            }
        }

        if ui
            .button(format!("{} Open folder", crate::icons::FOLDER_OPEN))
            .on_hover_text(captures_dir().display().to_string())
            .clicked()
            && let Err(error) = open_captures_dir()
        {
            tracing::error!(
                target: "kooch_editor::profiler",
                %error,
                "could not open the captures folder",
            );
        }
    });

    if !recording {
        ui.add_space(4.0);
        ui.label(
            "Stopped. Nothing is being recorded — press Record to start. \
             The ▶/⏸ below is a different control: it freezes the view on one frame \
             while recording continues.",
        );
    }
}

/// What the panel shows while a measurement is running: numbers, no
/// picture. Costs a handful of labels against the flamegraph's
/// thousands of glyphs.
fn recording_summary(ui: &mut Ui, frame_view: &puffin::GlobalFrameView) {
    let view = frame_view.lock();
    let stats = view.stats();
    ui.label(format!(
        "Recording — {} frames, {:.1} MiB",
        view.all_uniq().count(),
        stats.bytes_of_ram_used() as f64 / (1024.0 * 1024.0),
    ));
    ui.label("The flamegraph is hidden while recording: drawing it costs more than most of what it measures. Press Stop to read the capture.");
}

/// Reads the newest `.puffin` in the captures folder.
fn load_latest_capture() -> std::io::Result<Option<puffin::FrameView>> {
    let dir = captures_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(None);
    };
    let newest = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "puffin"))
        .filter_map(|p| {
            let modified = p.metadata().and_then(|m| m.modified()).ok()?;
            Some((modified, p))
        })
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path);

    let Some(path) = newest else {
        return Ok(None);
    };
    let mut file = std::io::BufReader::new(std::fs::File::open(&path)?);
    let view =
        puffin::FrameView::read(&mut file).map_err(|e| std::io::Error::other(e.to_string()))?;
    tracing::info!(
        target: "kooch_editor::profiler",
        path = %path.display(),
        "loaded a profiler capture",
    );
    Ok(Some(view))
}

/// Where captures go: beside the editor's own configuration rather than inside the project.
fn captures_dir() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::Path::new(&home).join(".local/share"))
        })
        .unwrap_or_else(std::env::temp_dir);
    base.join("kooch").join("captures")
}

/// Writes the current history and returns where it landed.
fn save_capture(frame_view: &puffin::GlobalFrameView) -> std::io::Result<std::path::PathBuf> {
    save_view(&frame_view.lock())
}

/// Writes any view, whichever process produced it.
pub(super) fn save_view(view: &puffin::FrameView) -> std::io::Result<std::path::PathBuf> {
    let dir = captures_dir();
    std::fs::create_dir_all(&dir)?;

    let count = view.all_uniq().count();
    let path = dir.join(format!("kooch-{count}-frames.puffin"));
    let mut file = std::io::BufWriter::new(std::fs::File::create(&path)?);
    view.write(&mut file)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(path)
}

/// Opens the captures folder in the system's file manager.
fn open_captures_dir() -> std::io::Result<()> {
    let dir = captures_dir();
    std::fs::create_dir_all(&dir)?;

    #[cfg(target_os = "linux")]
    let program = "xdg-open";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(target_os = "windows")]
    let program = "explorer";

    std::process::Command::new(program).arg(&dir).spawn()?;
    Ok(())
}
