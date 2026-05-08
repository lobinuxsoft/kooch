//! Performance + debug-controls sidebar (#463).
//!
//! Vertical list of collapsable sections rendered as a right-edge
//! overlay inside the View panel. Each section toggles
//! independently — the artist closes the ones they don't care
//! about and the others stay glanceable next to the viewport.
//!
//! Sections (top to bottom):
//! 1. **Debug**            — meshlet debug-view dropdown + LOD threshold slider.
//!                           Used to live in the View toolbar; pulled here so
//!                           the toolbar stays focused on gizmo controls and
//!                           every viewport-specific knob is in one place.
//! 2. **Frame**            — FPS instant + avg, CPU frame ms, GPU frame ms
//! 3. **System**           — process CPU%, RAM RSS
//! 4. **Render**           — engine-tracked VRAM, draw calls
//! 5. **Meshlet pipeline** — instances uploaded, dispatch threads,
//!                           pool size + roots

use ome_render::meshlet::{MeshletDebugMode, MeshletLodSettings, MeshletRenderStats};

use crate::perf::EditorPerfStats;

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_performance_content(
    ui: &mut egui::Ui,
    perf_stats: EditorPerfStats,
    meshlet_stats: MeshletRenderStats,
    meshlet_debug_mode: &mut MeshletDebugMode,
    meshlet_lod_settings: &mut MeshletLodSettings,
) {
    // auto_shrink=[true, true] lets the ScrollArea report only the
    // height its content actually needs, so the surrounding Frame
    // sizes to the visible sections (collapsing one shrinks the
    // dark container too). Width is bounded by the parent's
    // `set_max_width`; vertical scroll kicks in only when the
    // sections together exceed the viewport.
    egui::ScrollArea::vertical()
        .auto_shrink([true, true])
        .show(ui, |ui| {
            collapsing(ui, "Debug", true, |ui| {
                debug_controls(ui, meshlet_debug_mode, meshlet_lod_settings);
            });

            collapsing(ui, "Frame", true, |ui| {
                grid(ui, "perf_grid_frame", |ui| {
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
                        .unwrap_or_else(|| "n/a".to_string());
                    metric(ui, "GPU frame time", &gpu_text);
                });
            });

            collapsing(ui, "System", true, |ui| {
                grid(ui, "perf_grid_system", |ui| {
                    // {:.2} so sub-1 % (typical for an idle editor
                    // at 60 FPS waiting on vsync) is visible
                    // instead of rounding to 0.0 %.
                    metric(
                        ui,
                        "CPU usage (process)",
                        &format!("{:.2} %", perf_stats.cpu_percent),
                    );
                    metric(
                        ui,
                        "RAM (resident set)",
                        &format!("{} MB", perf_stats.ram_rss_mb()),
                    );
                });
            });

            collapsing(ui, "Render", true, |ui| {
                grid(ui, "perf_grid_render", |ui| {
                    metric(
                        ui,
                        "VRAM (engine-tracked)",
                        &format!("{} MB", perf_stats.vram_tracked_mb()),
                    );
                    metric_with_tooltip(
                        ui,
                        "Draw calls / frame",
                        &perf_stats.draw_calls.to_string(),
                        // #492 audit: explain why the empty-scene floor
                        // is non-zero so the artist doesn't read it as
                        // a leak.
                        "Editor base passes (sky + viewport blit + egui paint = 3) \
                         plus the meshlet stage's per-frame draw count \
                         (0 = empty scene, 4 = R64 atomic vbuf path, \
                         6 = R32 + Hi-Z 2-pass path). \
                         MeshRenderer.visible = false drops the entity at sync \
                         time, so an invisible mesh never reaches the cull \
                         pipeline and never bumps this number.",
                    );
                });
            });

            collapsing(ui, "Meshlet pipeline", true, |ui| {
                grid(ui, "perf_grid_meshlet", |ui| {
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
                });
            });
        });
}

fn debug_controls(
    ui: &mut egui::Ui,
    meshlet_debug_mode: &mut MeshletDebugMode,
    meshlet_lod_settings: &mut MeshletLodSettings,
) {
    ui.horizontal(|ui| {
        ui.label("Debug:");
        egui::ComboBox::from_id_salt("perf_debug_mode_combo")
            .selected_text(meshlet_debug_mode.label())
            .show_ui(ui, |ui| {
                for mode in MeshletDebugMode::all_implemented() {
                    ui.selectable_value(meshlet_debug_mode, *mode, mode.label());
                }
            })
            .response
            .on_hover_text(
                "Meshlet pipeline visualization mode. Off = production shading.",
            );
    });
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("LOD ≤").small())
            .on_hover_text("Pixel-error threshold for the continuous-LOD selector.");
        ui.add(
            egui::DragValue::new(&mut meshlet_lod_settings.target_error_pixels)
                .speed(0.05)
                .range(0.1_f32..=50.0_f32)
                .max_decimals(2)
                .suffix(" px"),
        )
        .on_hover_text(
            "Lower values keep more meshlets at any given distance. \
             Crank this up to force coarser LOD selection and visually \
             confirm the chain is being descended.",
        );
    });
}

/// Default-open collapsing header — section toggles with the chevron
/// next to the title. Persists across frames via egui's `Id`-keyed
/// state so the artist's preferences survive editor reloads.
fn collapsing(
    ui: &mut egui::Ui,
    title: &str,
    default_open: bool,
    body: impl FnOnce(&mut egui::Ui),
) {
    egui::CollapsingHeader::new(egui::RichText::new(title).strong())
        .id_salt(format!("perf_section_{title}"))
        .default_open(default_open)
        .show(ui, body);
}

/// Two-column grid for label / value rows.
fn grid(ui: &mut egui::Ui, salt: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(salt)
        .num_columns(2)
        .spacing([12.0, 4.0])
        .striped(true)
        .show(ui, body);
}

/// One label / value row inside a section grid.
fn metric(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(label);
    ui.label(egui::RichText::new(value).monospace());
    ui.end_row();
}

/// Same as [`metric`] but attaches `tooltip` on hover for both the label
/// and the value, used when the metric's number alone is misleading
/// without context (e.g. fixed editor passes that produce a non-zero
/// floor in an empty scene).
fn metric_with_tooltip(ui: &mut egui::Ui, label: &str, value: &str, tooltip: &str) {
    ui.label(label).on_hover_text(tooltip);
    ui.label(egui::RichText::new(value).monospace())
        .on_hover_text(tooltip);
    ui.end_row();
}
