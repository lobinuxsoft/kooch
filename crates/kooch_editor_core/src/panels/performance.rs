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
//! 6. **CPU frame**        — where `cpu_frame_ms` went, stage by stage,
//!                           plus what no stage claims. Collapsed by
//!                           default: it is a diagnostic, opened when a
//!                           number above looks wrong.
//! 7. **Remote**           — cost of the snapshot pull, split by
//!                           transport / decode. Hidden in local mode.

use kooch_lighting::{ClusterGrid, ClusterSettings, LightsHot};
use kooch_render::meshlet::{
    MeshletDebugCaps, MeshletDebugMode, MeshletLodSettings, MeshletRenderStats,
};

use crate::perf::EditorPerfStats;

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_performance_content(
    ui: &mut egui::Ui,
    perf_stats: EditorPerfStats,
    meshlet_stats: MeshletRenderStats,
    meshlet_debug_mode: &mut MeshletDebugMode,
    meshlet_debug_caps: MeshletDebugCaps,
    meshlet_lod_settings: &mut MeshletLodSettings,
    lights_hot: &mut LightsHot,
    cluster_settings: &mut ClusterSettings,
    viewport: egui::Vec2,
    hud_visibility: &mut crate::perf::HudVisibility,
    single_light_note: Option<&str>,
) {
    // auto_shrink=[true, true] lets the ScrollArea report only the
    // height its content actually needs, so the surrounding Frame
    // sizes to the visible sections (collapsing one shrinks the
    // dark container too). Width is bounded by the parent's
    // `set_max_width`; vertical scroll kicks in only when the
    // sections together exceed the viewport.
    egui::ScrollArea::vertical()
        .id_salt("performance_body")
        .auto_shrink([true, true])
        .show(ui, |ui| {
            collapsing(ui, "Debug", true, |ui| {
                debug_controls(
                    ui,
                    meshlet_debug_mode,
                    meshlet_debug_caps,
                    meshlet_lod_settings,
                    lights_hot,
                    cluster_settings,
                    viewport,
                    single_light_note,
                );
            });

            collapsing(ui, "Frame", true, |ui| {
                grid(ui, "perf_grid_frame", |ui| {
                    metric(
                        ui,
                        "FPS (instant)",
                        &format!("{:.0}", perf_stats.fps_instant),
                    );
                    metric(
                        ui,
                        "FPS (60-frame avg)",
                        &format!("{:.0}", perf_stats.fps_avg),
                    );
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
                    // #252 — per-pass GPU ms breakdown. Labels are
                    // path-specific: R64 emits `["Cull", "Raster",
                    // "Overlay"]`; the Hi-Z 2-pass orchestrator emits
                    // `["Pass A", "Hi-Z", "Pass B"]`. Sum equals
                    // `GPU frame time` above. `None` until the first
                    // ring readback completes (1-2 frames after
                    // `enable_gpu_timers`) or on adapters without
                    // `TIMESTAMP_QUERY`.
                    if let Some(stages) = meshlet_stats.stage_timings {
                        for (label, ms) in stages.iter() {
                            metric(ui, &format!("  · {label}"), &format!("{ms:.3} ms"));
                        }
                    }
                });
            });

            // #699 — the process that is actually simulating, next to the
            // editor's own frame because that is the comparison being
            // made: everything else on this panel describes the editor,
            // which is not what a person pressing Play is asking about.
            if let Some(host) = perf_stats.host {
                collapsing(ui, "Project", true, |ui| {
                    grid(ui, "perf_grid_host", |ui| {
                        metric_with_tooltip(
                            ui,
                            "Ticks (instant)",
                            &format!("{:.0} /s", host.ticks_instant),
                            "How many times a second the project's own process runs \
                             its update, from the last tick alone. Not frames per \
                             second: the host has no window and no renderer — the \
                             editor draws its world. This is the number that says \
                             whether the gameplay and the solver keep up.",
                        );
                        metric_with_tooltip(
                            ui,
                            "Ticks (60-tick avg)",
                            &format!("{:.0} /s", host.ticks_per_second),
                            "The same rate over the host's last sixty ticks. Lags \
                             the instant reading after Play or Stop, which is what \
                             an average is for — and why both are here instead of \
                             one number that is sometimes each.",
                        );
                        metric_with_tooltip(
                            ui,
                            "Tick time",
                            &format!("{:.2} ms", host.frame_ms),
                            "Wall-clock between the project's ticks, waiting \
                             included. A paused project still ticks.",
                        );
                        metric_with_tooltip(
                            ui,
                            "  · work",
                            &format!("{:.2} ms", host.cpu_frame_ms),
                            "The part of the tick that was work rather than waiting. \
                             This is what grows when the scene gets heavier, and the \
                             one to watch while a project is playing.",
                        );
                    });
                });
            }

            // The only reader of the sysinfo poll, which costs 2.08 ms
            // every time it runs (#703). Recorded so `sys_metrics_system`
            // can skip the frames nobody is reading.
            hud_visibility.system_section = collapsing(ui, "System", true, |ui| {
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
                    // #454.6 — per-stage cull survivor counts. Only
                    // populated when a debug-active mode is selected
                    // (any reject-overlay variant); the readback ring
                    // is skipped on production frames so the field
                    // stays None and the rows hide.
                    if let Some([after_frustum, after_backface, after_hi_z, total_visible]) =
                        meshlet_stats.cull_stage_counts
                    {
                        metric(ui, "After frustum", &after_frustum.to_string());
                        metric(ui, "After backface", &after_backface.to_string());
                        metric(ui, "After Hi-Z", &after_hi_z.to_string());
                        metric(ui, "Total visible", &total_visible.to_string());
                    }
                });
            });

            collapsing(ui, "CPU frame", false, |ui| {
                let breakdown = perf_stats.breakdown;
                let render = breakdown.render;
                grid(ui, "perf_grid_breakdown", |ui| {
                    metric_with_tooltip(
                        ui,
                        "Gather",
                        &format!("{:.2} ms", render.gather_ms),
                        "Building the frame's view of the world for the UI: hierarchy, \
                         inspector data, asset catalog. Walks every entity, so it grows \
                         with the scene.",
                    );
                    let gather = render.gather;
                    metric_with_tooltip(
                        ui,
                        "  · entities",
                        &format!("{:.2} ms", gather.entities_ms),
                        "Every entity with its components and their reflected field \
                         values. Grows with the world twice over — entities times \
                         components — and is paid whether or not a panel is open to \
                         read it.",
                    );
                    metric_with_tooltip(
                        ui,
                        "  · types",
                        &format!("{:.2} ms", gather.types_ms),
                        "The registered-type lists behind Add Component. Scales with \
                         the number of component types, not with the scene.",
                    );
                    metric_with_tooltip(
                        ui,
                        "  · archetypes",
                        &format!("{:.2} ms", gather.archetypes_ms),
                        "The archetype list for the Components panel.",
                    );
                    metric_with_tooltip(
                        ui,
                        "  · intern",
                        &format!("{:.2} ms", gather.intern_ms),
                        "Resolving every registered component name to a stable id, \
                         before the gathers above can use one.",
                    );
                    metric_with_tooltip(
                        ui,
                        "  · assets",
                        &format!("{:.2} ms", gather.assets_ms),
                        "The asset catalog for the Inspector's pickers, plus the \
                         contents of whatever the Asset Browser has selected.",
                    );
                    metric_with_tooltip(
                        ui,
                        "  · rest",
                        &format!("{:.2} ms", (render.gather_ms - gather.total_ms()).max(0.0)),
                        "The open scenes and the resource shuffling around the gathers \
                         — what gather spends outside the rows above.",
                    );
                    metric_with_tooltip(
                        ui,
                        "UI pass",
                        &format!("{:.2} ms", render.ui_ms),
                        "Laying out and painting every panel. egui is immediate mode: a \
                         list of 600 rows costs 600 rows every frame, whether or not one \
                         of them changed. Collapsing the panels is the quickest way to \
                         confirm this number.",
                    );
                    metric_with_tooltip(
                        ui,
                        "Input",
                        &format!("{:.2} ms", render.input_ms),
                        "Gizmo handles, viewport picking, camera. Near zero unless the \
                         pointer is doing something — which is exactly when it matters.",
                    );
                    metric_with_tooltip(
                        ui,
                        "Viewport",
                        &format!("{:.2} ms", render.viewport_ms),
                        "Recording the viewport's GPU commands — sky, meshlets, gizmos, \
                         blit. CPU-side encoding only; what the GPU then spends is the \
                         GPU frame row above.",
                    );
                    metric_with_tooltip(
                        ui,
                        "Present",
                        &format!("{:.2} ms", render.present_ms),
                        "Handing the frame to the surface, including egui's tessellation \
                         and texture uploads. With vsync on this also absorbs the wait \
                         for the vblank.",
                    );
                    metric_with_tooltip(
                        ui,
                        "Actions",
                        &format!("{:.2} ms", render.actions_ms),
                        "Applying what the UI queued: spawns, despawns, edits, saves. \
                         Zero on a frame where the user did nothing.",
                    );
                    metric_with_tooltip(
                        ui,
                        "Unaccounted",
                        &format!("{:.2} ms", breakdown.residual_ms(perf_stats.cpu_frame_ms)),
                        "CPU frame time minus the rows above. Near zero means the split \
                         describes the frame and the biggest row is the thing to fix. \
                         Large means the split is in the wrong place — the next stage \
                         boundary belongs inside whatever these rows are missing.",
                    );
                    metric_with_tooltip(
                        ui,
                        "Gizmo batch",
                        &format!("{:.2} ms", breakdown.gizmo_batch_ms),
                        "Rebuilding the gizmo line and mesh batches, before the render \
                         system runs. NOT part of CPU frame time and deliberately not \
                         deducted above — it is listed here because it is per-frame cost \
                         that scales with the scene and was otherwise invisible.",
                    );
                });
            });

            // #645 — only with a session; local mode has no pull, and a
            // section of zeroes would read as "measured, costs nothing".
            if let Some(remote) = perf_stats.remote {
                collapsing(ui, "Remote", true, |ui| {
                    grid(ui, "perf_grid_remote", |ui| {
                        metric_with_tooltip(
                            ui,
                            "Snapshot pull",
                            &format!("{:.2} ms", remote.refresh_ms),
                            "Main-thread stall for one snapshot pull, paid out of this \
                             frame's budget. Every frame while playing, one frame in \
                             thirty while paused. Holds the last pull's value on the \
                             frames in between.",
                        );
                        metric_with_tooltip(
                            ui,
                            "  · transport",
                            &format!("{:.2} ms", remote.transport_ms),
                            "Socket open, request, and the block until the project \
                             answers. The project serves requests from a Stage::First \
                             system, so this is mostly the wait for its next frame \
                             boundary — not bandwidth. If this dominates, the fix is to \
                             stop doing it on the main thread.",
                        );
                        metric_with_tooltip(
                            ui,
                            "  · decode",
                            &format!("{:.2} ms", remote.decode_ms),
                            "Parsing the response. If this dominates, the payload is \
                             the problem and the fix is to send less of it — diff \
                             server-side rather than resend the whole scene.",
                        );
                        metric_with_tooltip(
                            ui,
                            "Mirror apply",
                            &format!("{:.2} ms", remote.mirror_ms),
                            "Rebuilding the snapshot into the editor's own ECS, on the \
                             same frames as the pull. Reads 0.00 when the project \
                             reported nothing new — the mirror already matches the \
                             world, so there is nothing to walk.",
                        );
                        metric(ui, "Entities mirrored", &remote.entities.to_string());
                        metric(
                            ui,
                            "Snapshot size",
                            &format!("{:.1} KB", remote.snapshot_bytes as f32 / 1024.0),
                        );
                    });
                });
            }
        });
}

fn debug_controls(
    ui: &mut egui::Ui,
    meshlet_debug_mode: &mut MeshletDebugMode,
    meshlet_debug_caps: MeshletDebugCaps,
    meshlet_lod_settings: &mut MeshletLodSettings,
    lights_hot: &mut LightsHot,
    cluster_settings: &mut ClusterSettings,
    viewport: egui::Vec2,
    single_light_note: Option<&str>,
) {
    ui.horizontal(|ui| {
        ui.label("Debug:");
        egui::ComboBox::from_id_salt("perf_debug_mode_combo")
            .selected_text(meshlet_debug_mode.label())
            .show_ui(ui, |ui| {
                for mode in MeshletDebugMode::all_available_with_caps(&meshlet_debug_caps) {
                    ui.selectable_value(meshlet_debug_mode, mode, mode.label());
                }
            })
            .response
            .on_hover_text("Meshlet pipeline visualization mode. Off = production shading.");
    });
    // What the isolated light actually casts (#743). A point light with
    // no shadow renders exactly like one whose shadow broke, and the
    // view has nothing to draw that would tell them apart — so the
    // limitation is written down instead of left to be inferred.
    if *meshlet_debug_mode == MeshletDebugMode::SingleLight {
        match single_light_note {
            Some(note) => {
                ui.label(egui::RichText::new(note).small().weak())
                    .on_hover_text(
                        "Only directional lights have a shadow map today. Contact shadows are \
                         per light and off by default on point and spot.",
                    );
            }
            None => {
                ui.label(
                    egui::RichText::new("Select a light in the World panel")
                        .small()
                        .weak(),
                );
            }
        }
    }
    // The scale is a control, not a caption. A heatmap's top of scale is
    // the one number that decides whether the picture says anything: at
    // 16 a hundred-light stress scene is flat red and at 40 the same
    // frame separates into froxels. Fixed *during* a comparison, movable
    // between them — two screenshots at different tops mean nothing.
    if *meshlet_debug_mode == MeshletDebugMode::LightsPerPixel {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("red at ≥").small());
            ui.add(
                egui::DragValue::new(&mut lights_hot.0)
                    .speed(1.0)
                    .range(1..=256)
                    .suffix(" lights"),
            )
            .on_hover_text(
                "Top of the colour scale. Raise it until the picture stops being flat: \
                 that value is roughly how many lights the busiest froxel carries.",
            );
        });
        ui.label(
            egui::RichText::new(format!(
                "black 0 · blue few · green {} · red {}+",
                lights_hot.0 / 2,
                lights_hot.0
            ))
            .small()
            .weak(),
        )
        .on_hover_text(
            "Lights evaluated per pixel, directional included. A froxel's own count, read \
             where the shading loop pays it. Whole screen at full red with the scale raised \
             means the frame is shading without the cluster grid — every light for every pixel.",
        );
    }
    // The grid's reach, beside the view that shows what it costs (#820).
    //
    // A light is charged to every pixel of every cell it touches, so a
    // slice deeper than the light it holds spreads that light across
    // depth it never lit. The measurement that opened this: the busiest
    // froxel charged 40 lights where 14 reach the point.
    if *meshlet_debug_mode == MeshletDebugMode::LightsPerPixel {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("grid reaches").small());
            ui.add(
                egui::DragValue::new(&mut cluster_settings.far)
                    .speed(1.0)
                    .range(cluster_settings.first_slice.max(1.0) + 1.0..=1000.0)
                    .suffix(" m"),
            )
            .on_hover_text(
                "How far the froxel grid reaches. The same slices cover it however far \
                 it is, so a nearer far plane makes every one of them thinner. A light \
                 beyond it lands in the last slice with everything behind it — nothing \
                 renders wrong, but that slice over-lists.",
            );
        });
        // What the number means, which the number itself does not say.
        let grid = ClusterGrid::new(cluster_settings, glam::Vec2::new(viewport.x, viewport.y));
        ui.label(
            egui::RichText::new(format!(
                "{}×{}×{} cells · froxel {:.1} m deep at 15 m",
                grid.dimensions.x,
                grid.dimensions.y,
                grid.dimensions.z,
                grid.slice_depth(15.0),
            ))
            .small()
            .weak(),
        )
        .on_hover_text(
            "Compare that depth against the range of your lights. A froxel deeper than \
             the light it holds is what makes a pixel pay for lights that never reach it.",
        );
    }
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("LOD ≤").small())
            .on_hover_text("Pixel-error threshold for the continuous-LOD selector.");
        ui.add(
            crate::numeric::drag(&mut meshlet_lod_settings.target_error_pixels)
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
/// One collapsible section. Returns whether its body was drawn.
///
/// The return value matters for exactly one section — **System**, whose
/// numbers cost 2.08 ms to take (#703) — but is returned for all of them
/// rather than special-casing one, so the next expensive metric can be
/// gated without changing this signature again.
fn collapsing(
    ui: &mut egui::Ui,
    title: &str,
    default_open: bool,
    body: impl FnOnce(&mut egui::Ui),
) -> bool {
    egui::CollapsingHeader::new(egui::RichText::new(title).strong())
        .id_salt(format!("perf_section_{title}"))
        .default_open(default_open)
        .show(ui, body)
        .body_returned
        .is_some()
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

/// Width of the perf sidebar overlay anchored to the right edge of
/// the panel. 260 px fits the widest "n/a (TIMESTAMP_QUERY
/// unavailable)" GPU-frame-time row without wrapping while leaving
/// room to read the actual image behind it.
pub(crate) const PERF_SIDEBAR_WIDTH: f32 = 260.0;
const TOOLBAR_BUTTON_SIZE: f32 = 28.0;
const TOOLBAR_PADDING: f32 = 6.0;
const TOOLBAR_OFFSET: egui::Vec2 = egui::vec2(8.0, 8.0);

/// Draws the vertical perf sidebar anchored to the right edge of a
/// panel, with its always-visible toggle chevron.
///
/// Lives beside the Game panel rather than the View: every number in it
/// — frame time, cull dispatch, pool meshlets, remote snapshot — is the
/// cost of drawing the game, and the View shows an authoring camera that
/// nobody ships.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_perf_sidebar(
    ui: &mut egui::Ui,
    panel_origin: egui::Pos2,
    available: egui::Vec2,
    perf_stats: crate::perf::EditorPerfStats,
    meshlet_stats: kooch_render::meshlet::MeshletRenderStats,
    meshlet_debug_mode: &mut kooch_render::meshlet::MeshletDebugMode,
    meshlet_debug_caps: kooch_render::meshlet::MeshletDebugCaps,
    meshlet_lod_settings: &mut kooch_render::meshlet::MeshletLodSettings,
    lights_hot: &mut LightsHot,
    cluster_settings: &mut ClusterSettings,
    viewport: egui::Vec2,
    hud_visibility: &mut crate::perf::HudVisibility,
    single_light_note: Option<&str>,
) {
    // State lives in `HudVisibility` rather than in egui memory: the
    // systems that pay for these metrics run in `PreRender` and cannot
    // read egui's memory, so a flag kept only there meant nothing could
    // ask whether anyone was looking (#703).
    let mut sidebar_visible = hud_visibility.sidebar;

    let panel_top_right =
        panel_origin + egui::vec2(available.x - TOOLBAR_OFFSET.x, TOOLBAR_OFFSET.y);

    // Toggle chevron — left-pointing when expanded (click to
    // collapse to the right), right-pointing when collapsed (click
    // to expand back). Always rendered so the user has a way back
    // even after hiding the panel.
    let toggle_size = egui::vec2(TOOLBAR_BUTTON_SIZE, TOOLBAR_BUTTON_SIZE);
    let toggle_pos = panel_top_right - egui::vec2(toggle_size.x, 0.0);
    let toggle_rect = egui::Rect::from_min_size(toggle_pos, toggle_size);
    let mut toggle_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(toggle_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    egui::Frame::new()
        .fill(egui::Color32::from_rgba_unmultiplied(20, 20, 24, 200))
        .corner_radius(egui::CornerRadius::same(6))
        .show(&mut toggle_ui, |ui| {
            let glyph = if sidebar_visible {
                "\u{27e9}"
            } else {
                "\u{27e8}"
            };
            let button = egui::Button::new(egui::RichText::new(glyph).size(16.0))
                .min_size(toggle_size)
                .fill(egui::Color32::TRANSPARENT)
                .stroke(egui::Stroke::NONE);
            let resp = ui.add(button).on_hover_text(if sidebar_visible {
                "Hide performance sidebar"
            } else {
                "Show performance sidebar"
            });
            if resp.clicked() {
                sidebar_visible = !sidebar_visible;
            }
        });

    if sidebar_visible {
        // Panel sits below the toggle chevron, anchored to the
        // right edge. max_rect height is bounded by the viewport
        // so the inner ScrollArea can clip when sections overflow;
        // auto_shrink in `draw_performance_content` keeps the
        // Frame tight around the actually-visible content so
        // collapsing every section doesn't leave a giant black
        // box on the viewport.
        let panel_top = toggle_pos.y + toggle_size.y + 4.0;
        let panel_max_height =
            (available.y - 2.0 * TOOLBAR_OFFSET.y - toggle_size.y - 4.0).max(0.0);
        let sidebar_max_rect = egui::Rect::from_min_size(
            egui::pos2(panel_top_right.x - PERF_SIDEBAR_WIDTH, panel_top),
            egui::vec2(PERF_SIDEBAR_WIDTH, panel_max_height),
        );
        let mut sidebar_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(sidebar_max_rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        egui::Frame::new()
            .fill(egui::Color32::from_rgba_unmultiplied(20, 20, 24, 200))
            .corner_radius(egui::CornerRadius::same(6))
            .inner_margin(egui::Margin::same(TOOLBAR_PADDING as i8))
            .show(&mut sidebar_ui, |ui| {
                ui.set_max_width(PERF_SIDEBAR_WIDTH - TOOLBAR_PADDING * 2.0);
                draw_performance_content(
                    ui,
                    perf_stats,
                    meshlet_stats,
                    meshlet_debug_mode,
                    meshlet_debug_caps,
                    meshlet_lod_settings,
                    lights_hot,
                    cluster_settings,
                    viewport,
                    hud_visibility,
                    single_light_note,
                );
            });
    }

    // Written after drawing, so it records what was actually on screen
    // this frame rather than what was asked for. A collapsed sidebar
    // leaves `system_section` at whatever it last was, which is correct:
    // an invisible section is not an open one, and `wants_system_metrics`
    // requires both.
    hud_visibility.sidebar = sidebar_visible;

    // Written after drawing, so it records what was actually on screen
    // this frame rather than what was asked for. A collapsed sidebar
    // leaves `system_section` at whatever it last was, which is correct:
    // an invisible section is not an open one, and `wants_system_metrics`
    // requires both.
    hud_visibility.sidebar = sidebar_visible;
}
