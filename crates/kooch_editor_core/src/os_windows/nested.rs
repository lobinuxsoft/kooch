//! The nested egui pass each torn-off panel is drawn in.

use egui::ImmediateViewport;
use egui_dock::TabViewer;

use super::{SharedLive, viewport_of};
use crate::systems::tab_viewer::EditorTabViewer;

/// Registers the pass egui calls for every immediate viewport. Once, at startup: egui keeps one
/// renderer per thread.
pub(crate) fn install(ctx: &egui::Context, live: SharedLive) {
    // 🔴 egui embeds viewports as windows inside the main one unless told otherwise; eframe turns
    // this off on native, and a backend of our own has to as well.
    ctx.set_embed_viewports(false);
    egui::Context::set_immediate_viewport_renderer(move |ctx, viewport| {
        run_nested(ctx, &live, viewport);
    });
}

/// Draws every open panel window, inside the main pass.
pub(crate) fn show(ctx: &egui::Context, live: &SharedLive, viewer: &mut EditorTabViewer<'_>) {
    let open: Vec<_> = live
        .lock()
        .unwrap()
        .iter()
        .filter(|l| !l.closing)
        .map(|l| l.tab)
        .collect();
    for mut tab in open {
        let builder = egui::ViewportBuilder::default().with_title(super::title_of(tab));
        ctx.show_viewport_immediate(viewport_of(tab), builder, |ui, _| {
            egui::CentralPanel::default().show(ui, |ui| viewer.ui(ui, &mut tab));
        });
    }
}

fn run_nested(ctx: &egui::Context, live: &SharedLive, viewport: ImmediateViewport<'_>) {
    let ImmediateViewport {
        ids,
        mut viewport_ui_cb,
        ..
    } = viewport;
    // The main pass's clock, so an animation reads the same time in every window.
    let time = ctx.input(|i| i.time);
    let input = {
        let mut list = live.lock().unwrap();
        list.iter_mut()
            .find(|l| viewport_of(l.tab) == ids.this)
            .map(|l| {
                egui_winit::update_viewport_info(&mut l.info, ctx, &l.window, false);
                let mut input = l.state.take_egui_input(&l.window);
                input.viewports = std::iter::once((ids.this, l.info.clone())).collect();
                input.time = Some(time);
                input
            })
    };
    // 🔴 The callback runs even without a window: egui panics when a renderer never calls it. No
    // lock is held across it — the panel it draws may itself reach for this list.
    let mut input = input.unwrap_or_else(|| egui::RawInput {
        viewport_id: ids.this,
        viewports: std::iter::once((ids.this, egui::ViewportInfo::default())).collect(),
        ..Default::default()
    });
    // 🔴 The main pass's, always: egui rebuilds the font atlas whenever a pass brings a different
    // one, and the main window then drew its text off an atlas the nested pass had replaced.
    input.max_texture_side = Some(ctx.input(|i| i.max_texture_side));
    let output = ctx.run_ui(input, |ui| viewport_ui_cb(ui));
    let mut list = live.lock().unwrap();
    if let Some(open) = list.iter_mut().find(|l| viewport_of(l.tab) == ids.this) {
        open.info.events.clear();
        open.output = Some(output);
    }
}
