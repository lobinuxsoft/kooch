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
//!                           pool size + roots, last camera position the LOD
//!                           selector saw

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
    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
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
            });

            collapsing(ui, "Render", true, |ui| {
                grid(ui, "perf_grid_render", |ui| {
                    metric(
                        ui,
                        "VRAM (engine-tracked)",
                        &format!("{} MB", perf_stats.vram_tracked_mb()),
                    );
                    metric(ui, "Draw calls / frame", &perf_stats.draw_calls.to_string());
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
                    let [cx, cy, cz] = meshlet_stats.cam_pos;
                    metric(
                        ui,
                        "Camera position",
                        &format!("({:.2}, {:.2}, {:.2})", cx, cy, cz),
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
