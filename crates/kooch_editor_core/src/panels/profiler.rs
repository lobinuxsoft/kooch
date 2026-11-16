//! The profiler panel (#785) — where the frame actually goes.

use egui::Ui;

#[cfg(feature = "profiling")]
mod local;
#[cfg(feature = "profiling")]
mod remote;
mod shaders;

pub(crate) use shaders::shader_cost;
#[cfg(feature = "profiling")]
pub use shaders::shader_costs_in;

/// Frames left before re-asking puffin for the full scope snapshot.
///
/// 🔴 Asking once, at the moment recording starts, does not work, and
/// the reason is a lost flag inside puffin:
///
/// ```text
/// let propagate_full_delta = std::mem::take(&mut self.propagate_all_scope_details);
/// ...
/// Err(Error::Empty) => return,   // frame had no scopes
/// ```
///
/// The flag is TAKEN before the frame is built, and a frame that comes
/// out empty returns early — carrying the request away with it. The
/// frame that closes right after recording is switched on is exactly
/// that empty frame, because it ran while scopes were off.
///
/// The symptom is a capture where every scope reads `scope#ScopeId(67)`
/// except the handful registered after the fact. It does not look
/// broken; it is just unusable.
///
/// So the request is repeated for the first couple of seconds of
/// recording. Two frames was not enough: a scope only registers the
/// first time it runs, so anything that happens once every few frames —
/// or the first time you open a panel — registers after the snapshot has
/// already gone out, and comes back nameless. Repeating it costs one
/// atomic and one bool per frame for two seconds, against a capture that
/// is silently unreadable.
///
/// ⚠️ [`remote`] has the same symptom for a **different** reason, and
/// this countdown does not fix it. See [`write_capture`].
#[cfg(feature = "profiling")]
pub(crate) static SNAPSHOT_COUNTDOWN: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(0);

/// Writes `view` as a `.puffin` whose first frame carries every scope name the view has seen.
///
/// 🔴 `FrameView::write` does not serialise the collection: names reach the file only inside the
/// `scope_delta` of the frame they arrived with, and a bounded view evicts that frame (#785). This
/// writes the whole collection instead, so the view can stay bounded (#1908).
#[cfg(feature = "profiling")]
pub fn write_capture(
    view: &puffin::FrameView,
    out: &mut impl std::io::Write,
) -> anyhow::Result<()> {
    out.write_all(b"PUF0")?;
    let mut names = Some(view.scope_collection());
    for frame in view.all_uniq() {
        frame.write_into(names.take(), out)?;
    }
    Ok(())
}

/// Closes a game connection, if any: the panel is going away.
pub fn close() {
    #[cfg(feature = "profiling")]
    remote::disconnect();
}

/// Which process the panel is showing.
#[cfg(feature = "profiling")]
static SHOW_REMOTE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Draws the profiler, or the reason there is none.
pub fn draw_profiler_content(
    ui: &mut Ui,
    catalog: &[crate::panels::inspector::AssetCatalogEntry],
    shader_costs: &[(String, f32)],
) {
    #[cfg(feature = "profiling")]
    {
        use std::sync::atomic::Ordering;

        let mut remote_selected = SHOW_REMOTE.load(Ordering::Relaxed);
        ui.horizontal(|ui| {
            ui.label("Profiling:");
            ui.selectable_value(&mut remote_selected, false, "This editor")
                .on_hover_text("The process drawing this window, on this machine");
            ui.selectable_value(&mut remote_selected, true, "A running game")
                .on_hover_text(
                    "A game built with --features profiling, over the network. \
                     The only source that can answer the 10 W question.",
                );
        });
        SHOW_REMOTE.store(remote_selected, Ordering::Relaxed);
        ui.separator();

        // Each source's own shaders: the table under "A running game" is the game's, never this editor's.
        if remote_selected {
            remote::draw(ui, catalog);
        } else {
            shaders::draw(ui, &shaders::named_shader_costs(catalog, shader_costs));
            local::draw(ui);
        }
    }

    #[cfg(not(feature = "profiling"))]
    {
        let _ = (catalog, shader_costs);
        ui.heading("This editor was built without its profiler");
        ui.add_space(8.0);
        ui.label(
            "That is not the normal state: the feature is on by default, because an editor \
             that cannot answer \"why is this frame slow\" is missing the tool this one is \
             built around. Something passed --no-default-features.",
        );
        ui.add_space(8.0);
        ui.code("cargo run -p kooch_editor");
        ui.add_space(8.0);
        ui.label(
            "A shipped game is the opposite case and stays that way: its instrumentation is \
             opt-in, so a release carries none of it — not switched off, absent (#558).",
        );
        ui.add_space(8.0);
        ui.label(
            "To profile a game on the target hardware, build it with the profiling preset \
             from the Build panel and connect to it from the desktop.",
        );
    }
}
