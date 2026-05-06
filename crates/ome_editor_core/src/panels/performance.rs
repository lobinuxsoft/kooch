//! Performance panel — vertical perf + render diagnostics (#463).
//!
//! Replaces the toolbar-glued perf cluster + meshlet stats overlay
//! that #463.7 originally landed in the View toolbar. Vertical
//! sectioned layout in its own dockable tab so the viewport
//! toolbar stays focused on controls (gizmo mode, debug dropdown,
//! LOD slider) and the diagnostics get the room they need.
//!
//! Sections (top to bottom):
//! 1. **Frame**  — FPS instant + avg, CPU frame ms, GPU frame ms
//! 2. **System** — process CPU%, RAM RSS
//! 3. **Render** — engine-tracked VRAM, draw calls
//! 4. **Meshlet pipeline** — instances uploaded, dispatch threads,
//!    pool size + roots, last camera position the LOD selector saw
//!
//! All read-only — every value is populated by per-metric systems
//! wired in `EditorPlugin::build`. The widget is pure formatting.

use ome_render::meshlet::MeshletRenderStats;

use crate::perf::EditorPerfStats;

pub(crate) fn draw_performance_content(
    ui: &mut egui::Ui,
    perf_stats: EditorPerfStats,
    meshlet_stats: MeshletRenderStats,
) {
    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            section(ui, "Frame", |ui| {
                metric(ui, "FPS (instant)", &format!("{:.0}", perf_stats.fps_instant));
                metric(ui, "FPS (60-frame avg)", &format!("{:.0}", perf_stats.fps_avg));
                metric(
                    ui,
                    "CPU frame time",
                    &format!("{:.2} ms", perf_stats.cpu_frame_ms),
                );
                let gpu_text = perf_stats
                    .gpu_frame_ms
                    .map(|ms| format!("{:.2} ms", ms))
                    .unwrap_or_else(|| "n/a (TIMESTAMP_QUERY unavailable)".to_string());
                metric(ui, "GPU frame time", &gpu_text);
            });

            section(ui, "System", |ui| {
                metric(
                    ui,
                    "CPU usage (process)",
                    &format!("{:.1} %", perf_stats.cpu_percent),
                );
                metric(
                    ui,
                    "RAM (resident set)",
                    &format!("{} MB", perf_stats.ram_rss_mb()),
                );
            });

            section(ui, "Render", |ui| {
                metric(
                    ui,
                    "VRAM (engine-tracked)",
                    &format!("{} MB", perf_stats.vram_tracked_mb()),
                );
                metric(ui, "Draw calls / frame", &perf_stats.draw_calls.to_string());
            });

            section(ui, "Meshlet pipeline", |ui| {
                metric(
                    ui,
                    "Instances uploaded",
                    &meshlet_stats.instances_uploaded.to_string(),
                );
                metric(
                    ui,
                    "Cull dispatch threads",
                    &meshlet_stats.cull_threads.to_string(),
                );
                metric(
                    ui,
                    "Pool meshlets (total)",
                    &meshlet_stats.pool_meshlets_total.to_string(),
                );
                metric(
                    ui,
                    "Pool meshlets (roots)",
                    &meshlet_stats.pool_meshlets_roots.to_string(),
                );
                let [cx, cy, cz] = meshlet_stats.cam_pos;
                metric(
                    ui,
                    "Camera position",
                    &format!("({:.2}, {:.2}, {:.2})", cx, cy, cz),
                );
            });

            ui.add_space(8.0);
            ui.separator();
            ui.label(
                egui::RichText::new(
                    "VRAM is engine-tracked (GlobalMeshPool + render targets); does \
                     NOT include driver overhead, swap chain, or descriptor heaps. \
                     Adapter-side queries need per-backend native APIs that no \
                     portable Rust crate exposes today.",
                )
                .small()
                .italics()
                .color(egui::Color32::from_gray(180)),
            );
        });
}

/// Section header + framed group around its body — matches the
/// inspector's collapsing-style aesthetic without the collapse
/// affordance (perf data is meant to be glance-able, not hidden).
fn section(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new(title).strong());
    ui.separator();
    egui::Grid::new(format!("perf_grid_{title}"))
        .num_columns(2)
        .spacing([12.0, 4.0])
        .striped(true)
        .show(ui, body);
    ui.add_space(8.0);
}

/// One label / value row inside a section grid.
fn metric(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(label);
    ui.label(egui::RichText::new(value).monospace());
    ui.end_row();
}
