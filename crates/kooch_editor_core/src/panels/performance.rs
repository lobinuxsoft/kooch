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

use kooch_lighting::{ClusterGrid, ClusterSettings, LightsHot, SpecularFloor};
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
    specular_floor: &mut SpecularFloor,
    viewport: egui::Vec2,
    hud_visibility: &mut crate::perf::HudVisibility,
    single_light_note: Option<&str>,
    surface: PerfSurface,
) {
    // The panel scrolls; the overlay stack does not — a scroll area
    // inside the viewport-anchored column reserved space its cards did
    // not use, which read as panels "longer than their content".
    let scroll = |ui: &mut egui::Ui, body: Box<dyn FnOnce(&mut egui::Ui) + '_>| {
        if surface == PerfSurface::Panel {
            egui::ScrollArea::vertical()
                .id_salt("performance_body")
                .auto_shrink([true, true])
                .show(ui, body);
        } else {
            body(ui);
        }
    };
    scroll(
        ui,
        Box::new(|ui| {
            let mut pin_debug = hud_visibility.pinned.debug;
            section(ui, &mut pin_debug, surface, "Debug", true, |ui| {
                debug_controls(
                    ui,
                    meshlet_debug_mode,
                    meshlet_debug_caps,
                    meshlet_lod_settings,
                    lights_hot,
                    cluster_settings,
                    specular_floor,
                    meshlet_stats.cluster_occupancy,
                    viewport,
                    single_light_note,
                );
            });

            hud_visibility.pinned.debug = pin_debug;

            // The shadow-pages readout, a section like any other: in
            // the tab it collapses, on the viewport it is one more
            // card in the stack. The floating window it once was is
            // retired — the stack IS the way overlays live now.
            let mut pin_pages = hud_visibility.shadow_pages_window;
            section(ui, &mut pin_pages, surface, "Shadow pages", true, |ui| {
                shadow_page_readout(ui, meshlet_stats.page_marking, meshlet_stats.page_raster);
            });
            hud_visibility.shadow_pages_window = pin_pages;

            section(
                ui,
                &mut hud_visibility.pinned.frame,
                surface,
                "Frame",
                true,
                |ui| {
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
                },
            );

            // #699 — the process that is actually simulating, next to the
            // editor's own frame because that is the comparison being
            // made: everything else on this panel describes the editor,
            // which is not what a person pressing Play is asking about.
            if let Some(host) = perf_stats.host {
                section(
                    ui,
                    &mut hud_visibility.pinned.project,
                    surface,
                    "Project",
                    true,
                    |ui| {
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
                    },
                );
            }

            // The only reader of the sysinfo poll, which costs 2.08 ms
            // every time it runs (#703). Recorded so `sys_metrics_system`
            // can skip the frames nobody is reading.
            let mut pin_system = hud_visibility.pinned.system;
            let system_shown = section(ui, &mut pin_system, surface, "System", true, |ui| {
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

            hud_visibility.pinned.system = pin_system;
            // Only the PANEL surface votes on the section's openness:
            // the overlay writes the same flag through `pinned.system`,
            // and letting it overwrite here would turn the poll off
            // while the tab still shows the numbers.
            if surface == PerfSurface::Panel {
                hud_visibility.system_section = system_shown;
            }
            section(
                ui,
                &mut hud_visibility.pinned.render,
                surface,
                "Render",
                true,
                |ui| {
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
                },
            );

            section(
                ui,
                &mut hud_visibility.pinned.meshlet,
                surface,
                "Meshlet pipeline",
                true,
                |ui| {
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
                },
            );

            section(
                ui,
                &mut hud_visibility.pinned.cpu_frame,
                surface,
                "CPU frame",
                false,
                |ui| {
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
                },
            );

            // #645 — only with a session; local mode has no pull, and a
            // section of zeroes would read as "measured, costs nothing".
            if let Some(remote) = perf_stats.remote {
                section(
                    ui,
                    &mut hud_visibility.pinned.remote,
                    surface,
                    "Remote",
                    true,
                    |ui| {
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
                    },
                );
            }
        }),
    );
}

/// Digit groups, because a pair-test count runs to seven figures and an
/// unbroken run of digits is a number nobody reads.
fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

fn debug_controls(
    ui: &mut egui::Ui,
    meshlet_debug_mode: &mut MeshletDebugMode,
    meshlet_debug_caps: MeshletDebugCaps,
    meshlet_lod_settings: &mut MeshletLodSettings,
    lights_hot: &mut LightsHot,
    cluster_settings: &mut ClusterSettings,
    specular_floor: &mut SpecularFloor,
    cluster_occupancy: Option<(u32, f32)>,
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
    // 🔴 The same problem the note above solves, for the two views that
    // shipped painting the whole screen one colour: a code the reader
    // has to remember is a code the reader does not have. Orange means
    // "pick a lamp", and saying so is one line.
    let lamp_view = matches!(
        *meshlet_debug_mode,
        MeshletDebugMode::LocalPageFaces | MeshletDebugMode::LocalPageDepth
    );
    if lamp_view {
        match single_light_note {
            Some(note) => {
                ui.label(egui::RichText::new(note).small().weak());
                let legend = if *meshlet_debug_mode == MeshletDebugMode::LocalPageFaces {
                    "6 hues = cube face · brightness = chain level · white = no page"
                } else {
                    "red = occluded · green = lit · blue = no page"
                };
                ui.label(egui::RichText::new(legend).small().weak())
                    .on_hover_text(
                        "One lamp at a time, because a hundred averaged together is the                          signal this view exists to show. Faces answers which page was                          READ; occlusion answers what that page CONTAINED — a wrong                          shadow is one or the other and no single view separates them.                          Black is outside the lamp's range; magenta is the paged shadow                          path switched off.",
                    );
            }
            None => {
                ui.label(
                    egui::RichText::new(
                        "Orange everywhere = select a point or spot light in the World panel",
                    )
                    .small()
                    .weak(),
                );
            }
        }
    }
    // A legend, because this view's whole value is that its three cases
    // are different faults and not different amounts of the same one.
    // Left to be inferred, blue reads as "very dark shadow".
    if *meshlet_debug_mode == MeshletDebugMode::PointShadowFactor {
        ui.label(
            egui::RichText::new(
                "grey = the cube's factor · blue = past range · magenta = no caster",
            )
            .small()
            .weak(),
        )
        .on_hover_text(
            "The cube map's answer with nothing on top of it: no BRDF, no cosine, no \
             exposure, no ambient, no second light. Black is fully occluded, white fully \
             lit. Select a point light in the World panel to ask about that one; otherwise \
             it answers for the strongest lamp reaching each pixel.",
        );
    }
    if *meshlet_debug_mode == MeshletDebugMode::PointCubeFaces {
        ui.label(
            egui::RichText::new("+X -X +Y / -Y +Z -Z · dark blue = nothing recorded")
                .small()
                .weak(),
        )
        .on_hover_text(
            "The cube map opened up, one cell per world axis. Dark blue is a face with no \
             occluder in it — what a caster culled out of the shadow pass looks like. The \
             grey ramp is distance to the recorded occluder over the lamp's range.",
        );
    }
    // The scale is a control, not a caption. A heatmap's top of scale is
    // the one number that decides whether the picture says anything: at
    // 16 a hundred-light stress scene is flat red and at 40 the same
    // frame separates into froxels. Fixed *during* a comparison, movable
    // between them — two screenshots at different tops mean nothing.
    if *meshlet_debug_mode == MeshletDebugMode::LightsPerPixel {
        // 🔴 The measurement, rather than a colour to squint at. Read
        // where the shading loop pays it, carried home by the readback
        // the grid already runs — bisecting the scale by eye does not
        // separate 32 from 45, and cannot compare before and after a
        // change to the grid without doing the bisection twice (#820).
        match cluster_occupancy {
            Some((peak, mean)) => {
                ui.label(
                    egui::RichText::new(format!("busiest froxel {peak} lights · mean {mean:.1}"))
                        .small(),
                )
                .on_hover_text(
                    "Counted on the GPU over every cell of the grid, a frame or two ago. \
                     The mean is over cells that hold at least one light, not over the \
                     empty half of the grid. Set the scale below to the peak and the \
                     picture uses its whole range.",
                );
            }
            None => {
                ui.label(
                    egui::RichText::new("froxel counts: not clustering this frame")
                        .small()
                        .weak(),
                )
                .on_hover_text(
                    "No camera matrices, clustering switched off, or the first readback \
                     has not landed yet (1-2 frames).",
                );
            }
        }
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
        // 🔴 Both ends, because the window is what matters and the near
        // one is the stronger lever: 24 slices spread over [5, 200] put
        // a 5.1 m froxel at 30 m, and over [20, 60] put a 1.4 m one
        // there. Exposing only `far` hid that the first fifteen metres
        // of grid were being spent on empty air in front of the camera.
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("grid from").small());
            ui.add(
                egui::DragValue::new(&mut cluster_settings.first_slice)
                    .speed(0.5)
                    .range(0.1..=(cluster_settings.far - 1.0).max(1.0))
                    .suffix(" m"),
            )
            .on_hover_text(
                "Where the first slice starts. Everything NEARER piles into slice 0 \
                 together, so raise it to the distance of your closest lit surface and \
                 no further.",
            );
        });
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("grid reaches").small());
            ui.add(
                egui::DragValue::new(&mut cluster_settings.far)
                    .speed(1.0)
                    .range((cluster_settings.first_slice + 1.0)..=1000.0)
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
        //
        // 🔴 Three distances, not one. A single sample invites picking
        // the flattering one, and the shape is the point: slices grow
        // logarithmically, so a grid that looks fine up close is coarse
        // at the far end — and everything past `far` piles into the last
        // slice, which is the way lowering it makes things *worse*.
        let grid = ClusterGrid::new(cluster_settings, glam::Vec2::new(viewport.x, viewport.y));
        let far = cluster_settings.far;
        ui.label(
            egui::RichText::new(format!(
                "{}×{}×{} cells (this Game view) · froxel {} / {} / {} deep at 10 / 25 / {:.0} m",
                grid.dimensions.x,
                grid.dimensions.y,
                grid.dimensions.z,
                depth_label(grid.slice_depth(10.0)),
                depth_label(grid.slice_depth(25.0)),
                depth_label(grid.slice_depth(far * 0.9)),
                far * 0.9,
            ))
            .small()
            .weak(),
        )
        .on_hover_text(
            "Compare those depths against the range of your lights: a froxel deeper than \
             the light it holds makes a pixel pay for lights that never reach it. \
             🔴 Anything FURTHER than the grid reaches lands in the last slice together, \
             so a far plane nearer than your geometry over-lists instead of helping — set \
             it by the distance from the camera to the furthest lit surface, not by the \
             size of the scene. The cell count is this Game view's; the View panel has a \
             different aspect and therefore a different grid.",
        );
    }
    // #821 — the specular layer is the expensive half of the model, and
    // a light contributing a fraction of the frame's exposure spends all
    // of it on a highlight nobody can see. Zero is off, and off is what
    // every frame did before this existed.
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("diffuse-only under").small());
        ui.add(
            egui::DragValue::new(&mut specular_floor.0)
                .speed(0.5)
                .range(0.0..=10_000.0)
                .suffix(" lx"),
        )
        .on_hover_text(
            "Irradiance below which a light skips its specular layer — GGX, Smith, \
             Fresnel, multiscatter and the representative point. 0 keeps every light on \
             the full model. Raise it while watching the picture: the frame time falls \
             immediately, and the value to keep is the last one before highlights start \
             disappearing where anybody looks.",
        );
    });
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

/// The shadow-page marking pass and what it found (#866).
///
/// 🔴 A READOUT, not a control. #866 kept these as panel-only
/// diagnostics while nothing read what marking wrote — a knob promising
/// memory nobody spent. The pass decides which shadows exist now, so
/// every knob moved into the project's render settings, in a
/// `Shadows: virtual pages` group beside the cascades it replaces. What
/// stays here is what a settings file cannot hold: what the last frame
/// actually did.
fn shadow_page_readout(
    ui: &mut egui::Ui,
    page_counts: Option<kooch_render::shadow::pages::mark::MarkCounts>,
    raster_counts: Option<kooch_render::shadow::pages::raster::RasterCounts>,
) {
    use kooch_render::shadow::pages::PageConfig;

    let Some(counts) = page_counts else {
        ui.label(
            egui::RichText::new("pages: waiting for the first readback")
                .small()
                .weak(),
        );
        return;
    };

    if counts.overflow > 0 {
        ui.label(
            egui::RichText::new(format!(
                "{} pages past the buffer — every number below is a floor",
                counts.overflow
            ))
            .small()
            .color(egui::Color32::from_rgb(220, 120, 90)),
        );
    }

    // MiB, because pages are the unit and megabytes are the budget.
    let config = PageConfig::default();
    let mib = counts.resident as f64 * config.page_bytes() as f64 / (1024.0 * 1024.0);
    ui.label(
        egui::RichText::new(format!(
            "view {} · {} pages · {mib:.1} MiB · at {}x{}",
            counts.view, counts.resident, counts.size.0, counts.size.1
        ))
        .small(),
    )
    .on_hover_text(
        "Distinct pages the frame would make resident, at 128-texel pages and Depth32Float. \
         The resolution is part of the reading, not context: a page count without it is not \
         a number, and the View and Game tabs are two cameras at two sizes. \
         Read it against Unreal's own pool, which is 4096 pages for the WHOLE scene by \
         default (6144 for open worlds, 8192 thrashes) — and against this engine's 152 MiB \
         of fixed shadow allocations, which stand whether or not a light casts.",
    );
    // 🔴 The count is for EVERY light the grid holds, not the handful
    // that have a shadow slot today — and that is the measurement, not
    // an oversight. A virtual shadow map exists for many lights; counting
    // only the four that fit today's slots would be measuring the cap
    // the feature is meant to remove.
    ui.label(
        egui::RichText::new(if counts.culled > 0 {
            format!(
                "{} samples · {} sample/light pairs · {} gated by coverage",
                counts.samples, counts.pairs, counts.culled
            )
        } else {
            format!(
                "{} samples · {} sample/light pairs · every light casting",
                counts.samples, counts.pairs
            )
        })
        .small()
        .weak(),
    )
    .on_hover_text(
        "The pass walks the froxel grid, which holds every light that reaches a pixel — so \
         this is what the scene would cost with ALL of its lights casting. Pairs divided \
         by samples is the grid's own lights-per-pixel. Gated pairs are lights whose whole \
         range projects under `shadow_min_pixels` on screen (#944): they still shade, but \
         a shadow nobody can resolve claims no pages.",
    );
    // 🔴 What the same work would cost per FROXEL instead of per pixel.
    // Olsson §III derives shadow resolution and page masks from
    // cluster/light pairs rather than sample/light pairs, because
    // cluster bounds are "several orders of magnitude fewer than the
    // samples". This line is that claim, in this scene, as a number
    // rather than an argument (#952).
    if counts.froxels > 0 && counts.samples > 0 {
        let per_pixel = counts.pairs as f32 / counts.samples as f32;
        let froxel_pairs = (counts.froxels as f32 * per_pixel).max(1.0);
        ui.label(
            egui::RichText::new(format!(
                "{} froxels occupied · {:.1} lights each · {:.0}× fewer pairs per froxel",
                counts.froxels,
                per_pixel,
                counts.pairs as f32 / froxel_pairs,
            ))
            .small()
            .weak(),
        )
        .on_hover_text(
            "Froxels of this view that held visible surface, counted from the depth \
             buffer. The marking runs per (pixel, light); the same walk over occupied \
             froxels would run per (froxel, light), and this is the ratio between the \
             two. It is an upper bound on the win: a froxel's bounds project to a RANGE \
             of pages rather than one, so a cluster pass marks conservatively and spends \
             pool slots the per-pixel version never asked for.",
        );
    }
    // 🔴 The comparison that makes the number mean something, and it is
    // one budget for every light in the scene rather than per light.
    // It is now this engine's OWN pool rather than a figure quoted from
    // Epic: the allocator that hands the slots out is what reports the
    // capacity.
    ui.label(
        egui::RichText::new(format!(
            "{} of {} pages in this view's slice · {:.0}% full",
            counts.pool.allocated(),
            counts.pool.capacity,
            counts.pool.load()
        ))
        .small()
        .weak(),
    )
    .on_hover_text(
        "The physical pool the pages are allocated out of, in the same dispatch that marks \
         them: the thread that flips a page's mark bit is the one that claims its slot. \
         The pool is SLICED between the cameras — a layer of the atlas each — so this is \
         what THIS view may spend, not the whole budget: a camera cannot take another \
         camera's pages and cannot be robbed of its own. \
         Epic's default pool is 4096 pages for the WHOLE scene — 6144 for open worlds, 8192 \
         thrashes — and `KOOCH_SHADOW_POOL_PAGES` moves this one.",
    );
    // 🔴 The split that explains everything else on this panel. Marking
    // counts what a hundred casting lights WOULD need; only the sun's
    // pages are rasterised, so only they spend the pool. Before the two
    // were separated, local pages took 991 of each camera's 1024 slots
    // and the sun got 33 — a pool reporting itself full while doing
    // nothing.
    if counts.pool.denied > 0 {
        ui.label(
            egui::RichText::new(format!(
                "{} denied by rank — the plan funded down to rank {}",
                counts.pool.denied, counts.pool.cutoff
            ))
            .small()
            .color(egui::Color32::from_rgb(230, 190, 90)),
        )
        .on_hover_text(
            "The frame wanted more pages than this view's slice holds, so the seating plan \
             (#942) ranked the demand and funded it coarsest-first: the sun's clipmap ahead \
             of every local light, and within any chain the coarse levels ahead of the fine. \
             What was denied is the finest detail, never a whole light's coverage — and the \
             number to shrink it is #943's resolution bias, not a bigger pool.",
        );
    }
    if counts.pool.bias_local > 0 || counts.pool.bias_sun > 0 {
        ui.label(
            egui::RichText::new(format!(
                "resolution bias: locals +{} · sun +{} levels",
                counts.pool.bias_local, counts.pool.bias_sun
            ))
            .small()
            .weak(),
        )
        .on_hover_text(
            "The demand did not fit the slice, so the marking asks coarser (#943): each \
             level is a quarter of the pages. Locals pay up to four levels before the sun \
             pays one, and it unwinds on its own when the demand shrinks. A bias that sits \
             high is the pool saying it is too small for the scene — raise \
             `shadow_pool_pages` or lower `shadow_density`.",
        );
    }
    if counts.pool.preempted > 0 {
        ui.label(
            egui::RichText::new(format!(
                "{} residents preempted — their seat went to a higher rank",
                counts.pool.preempted
            ))
            .small()
            .weak(),
        )
        .on_hover_text(
            "Pages evicted by PRESSURE rather than by age: the plan did not fund their rank \
             this frame. A camera that stopped moving should drive this to zero within a \
             frame — persistent churn here means the demand is oscillating around the \
             cutoff rank.",
        );
    }
    // 🔴 The reading persistence exists to produce. A still camera
    // should sit at 100 %: every page it wants is one it already has,
    // so the raster draws nothing and the atlas is last frame's.
    ui.label(
        egui::RichText::new(format!(
            "{} reused · {} new · {} evicted · {:.0}% hit",
            counts.pool.reused,
            counts.pool.claims,
            counts.pool.evicted,
            counts.pool.hit_rate()
        ))
        .small()
        .weak(),
    )
    .on_hover_text(
        "The pool PERSISTS between frames: a page is freed when nothing has asked for it in \
         `max_age` frames (60 by default — Epic's `MaxPageAgeSinceLastRequest`, \
         `KOOCH_SHADOW_PAGE_AGE` overrides it), or the moment the seating plan stops \
         funding its rank under pressure (#942). A reused page is one whose depth is \
         already in the atlas and does not have to be rasterised again; movement \
         invalidation re-lists what a caster passed through.",
    );
    if counts.pool.leaked > 0 {
        ui.label(
            egui::RichText::new(format!(
                "{} slots fell out of the free list — a double free",
                counts.pool.leaked
            ))
            .small()
            .color(egui::Color32::from_rgb(220, 120, 90)),
        )
        .on_hover_text(
            "The free list cannot hold more slots than the slice has, so this is the              allocator releasing a slot twice. Always zero, or the allocator is wrong.",
        );
    }
    if counts.pool.overflow > 0 {
        ui.label(
            egui::RichText::new(format!(
                "{} pages went unallocated — the pool is full",
                counts.pool.overflow
            ))
            .small()
            .color(egui::Color32::from_rgb(220, 120, 90)),
        )
        .on_hover_text(
            "Pages the frame needed and the pool could not give a slot to. They render \
             unshadowed. Epic's own pool overflow shows up as checkerboard corruption or \
             missing shadows, which is exactly the kind of failure nobody recognises by \
             sight — so it is named here instead.",
        );
    }
    if let Some(raster) = raster_counts {
        // 🔴 What was actually DRAWN, against what was asked for. The
        // marking count above is a request; this is the answer, and the
        // two differing is the single most useful thing this panel can
        // say.
        ui.label(
            egui::RichText::new(format!(
                "{} pages rastered · {} cached · {} meshlet/page pairs",
                raster.pages, raster.cached, raster.pairs
            ))
            .small()
            .weak(),
        )
        .on_hover_text(
            "The pages the depth raster actually filled, the resident pages whose \
             content survived from an earlier frame (the cache — those cost nothing), \
             and the meshlet/page pairs drawn to fill the dirty ones. A still scene \
             should raster near zero; UE5's rule of thumb is under 5% of residents.",
        );
        // 🔴 The number that decides the shape of the local-light
        // raster. The expansion is a product — a level's pages times a
        // level's surviving meshlets — so what it costs is the
        // combinations it walks, not the pairs it finds.
        if raster.tests > 0 {
            let per_pair = raster.tests as f32 / raster.pairs.max(1) as f32;
            ui.label(
                egui::RichText::new(format!(
                    "{} pair tests · {per_pair:.0} per pair · {} depth-rejected · worst level {} at {}",
                    thousands(raster.tests),
                    raster.depth_rejected,
                    raster.worst.0,
                    thousands(raster.worst.1)
                ))
                .small()
                .weak(),
            )
            .on_hover_text(
                "The expansion asks, for every page of a level and every meshlet that \
                 survived that level's cull, whether the two touch. So its cost is pages \
                 TIMES meshlets, and the pairs it emits are what is left after the \
                 question is answered — the ratio is how much of the pass is spent \
                 proving a miss. \
                 ⚠️ It is also the number that decides whether local lights are \
                 affordable: they multiply the page side by roughly eighty, and the \
                 inverse form — asking which pages a meshlet touches — was measured \
                 WORSE for the sun, because a meshlet's rect covers up to 16384 cells at \
                 the finest clipmap levels while only twenty pages are resident there.",
            );
            // 🔴 The counted cost of the shape this pass does NOT use.
            // Both numbers are measured every frame so the choice
            // between them is arithmetic instead of an opinion — which
            // is the thing that was missing the last time one of them
            // shipped everywhere at once.
            let save = raster.tests.saturating_sub(raster.hybrid);
            let cut = save as f32 / raster.tests.max(1) as f32 * 100.0;
            ui.label(
                egui::RichText::new(format!(
                    "scatter would run {} · per-level best {} ({cut:.0}% off)",
                    thousands(raster.scatter),
                    thousands(raster.hybrid),
                ))
                .small()
                .weak(),
            )
            .on_hover_text(
                "There are two ways to find which meshlet belongs in which page.                  PAIRING walks every resident page against every survivor, which is                  what runs today. SCATTERING walks the cells each meshlet's bounds                  cover and looks them up, which is what the first number would cost —                  counted here without being run.                  Neither wins everywhere: a page at level 0 is centimetres wide so one                  meshlet covers thousands of cells against a handful of resident pages,                  while at level 12 a page is hundreds of metres and every meshlet lands                  in exactly one cell.                  The second number takes the cheaper shape at each level separately, and                  the percentage is the whole prize a hybrid has to offer.",
            );
        }
        if raster.local > 0 {
            ui.label(
                egui::RichText::new(format!(
                    "{} local-light pages drawn",
                    thousands(raster.local as u64),
                ))
                .small()
                .weak(),
            )
            .on_hover_text(
                "Pages belonging to point and spot lights, rasterised this frame. They \
                 share the sun's buckets: a bucket is an OCTAVE of world texel size, so a \
                 lamp and the sun that want the same fineness draw from the same survivor \
                 list — which is what lets the local half cost no cull of its own. \
                 A page carries its light in its own key, so nothing downstream needs the \
                 list split by lamp; a bucket per light per level would be the 4848-view \
                 shape this exists to avoid. \
                 ⚠️ They spend the same pool the sun does. When the pool fills, the \
                 overflow line above says so.",
            );
        }
        if raster.dropped > 0 || raster.overflow > 0 {
            ui.label(
                egui::RichText::new(format!(
                    "{} pages dropped · {} pairs past the list — shadows are missing",
                    raster.dropped, raster.overflow
                ))
                .small()
                .color(egui::Color32::from_rgb(220, 120, 90)),
            );
        }
    }
    // The hash's two failure meters — tombstones walked and inserts out
    // of probes — are gone with the hash: the flat table has no probe
    // run to degrade. See `page_table.wgsl`.
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
/// The Performance dock tab (the user's ask): everything the overlay
/// sidebar showed, in a real panel off the game view, with every
/// section pinnable into its own floating window.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_performance_panel(
    ui: &mut egui::Ui,
    perf_stats: EditorPerfStats,
    meshlet_stats: MeshletRenderStats,
    meshlet_debug_mode: &mut MeshletDebugMode,
    meshlet_debug_caps: MeshletDebugCaps,
    meshlet_lod_settings: &mut MeshletLodSettings,
    lights_hot: &mut LightsHot,
    cluster_settings: &mut ClusterSettings,
    specular_floor: &mut SpecularFloor,
    game_viewport: Option<(u32, u32)>,
    hud_visibility: &mut crate::perf::HudVisibility,
    single_light_note: Option<&str>,
) {
    hud_visibility.panel_visible = true;
    // The cluster grid's shape needs the GAME viewport, which this tab
    // does not render — the Game tab's last size request stands in. The
    // fallback only matters with the Game tab closed, where the grid is
    // informational anyway.
    let viewport = game_viewport
        .map(|(w, h)| egui::vec2(w as f32, h as f32))
        .unwrap_or(egui::vec2(1920.0, 1080.0));
    draw_performance_content(
        ui,
        perf_stats,
        meshlet_stats,
        meshlet_debug_mode,
        meshlet_debug_caps,
        meshlet_lod_settings,
        lights_hot,
        cluster_settings,
        specular_floor,
        viewport,
        hud_visibility,
        single_light_note,
        PerfSurface::Panel,
    );
}

/// Which surface is rendering the sections: the Performance dock tab,
/// or the semi-transparent card stack on the game viewport.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum PerfSurface {
    Panel,
    Overlay,
}

/// One section of the readout.
///
/// On the PANEL every section renders, as a collapsing header whose pin
/// toggle mirrors it onto the game viewport. On the OVERLAY only the
/// toggled sections render, each as a semi-transparent card in the
/// stack, with a ✕ to dismiss — Godot's viewport overlays, the user's
/// ask. Returns whether the body rendered, which the System section's
/// poll gate reads.
fn section(
    ui: &mut egui::Ui,
    pinned: &mut bool,
    surface: PerfSurface,
    title: &str,
    default_open: bool,
    body: impl FnOnce(&mut egui::Ui),
) -> bool {
    if surface == PerfSurface::Overlay {
        if !*pinned {
            return false;
        }
        stack_card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(title).strong().small());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button(crate::icons::X)
                        .on_hover_text("Hide this overlay")
                        .clicked()
                    {
                        *pinned = false;
                    }
                });
            });
            body(ui);
        });
        return true;
    }
    let id = ui.make_persistent_id(format!("perf_section_{title}"));
    let state = egui::collapsing_header::CollapsingState::load_with_default_open(
        ui.ctx(),
        id,
        default_open,
    );
    let (_, _, body_response) = state
        .show_header(ui, |ui| {
            ui.label(egui::RichText::new(title).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut on = *pinned;
                if ui
                    .toggle_value(&mut on, crate::icons::MAP_PIN_SIMPLE_AREA)
                    .on_hover_text("Show as an overlay card on the game viewport")
                    .clicked()
                {
                    *pinned = on;
                }
            });
        })
        .body(body);
    body_response.is_some()
}

/// A semi-transparent card in the game viewport's overlay stack.
pub(crate) fn stack_card(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(egui::Color32::from_rgba_unmultiplied(20, 20, 24, 170))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(264.0);
            body(ui);
        });
    ui.add_space(6.0);
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

/// A froxel's depth as the panel should state it.
///
/// The two clamped ends are the ones that matter: an unbounded last
/// slice formats as `inf` and a scene sitting in front of the grid reads
/// as a suspiciously thin cell. Both get words, because both mean "your
/// geometry is outside the grid" and neither is a measurement.
fn depth_label(metres: f32) -> String {
    match metres.is_finite() {
        true => format!("{metres:.1} m"),
        false => "unbounded".to_owned(),
    }
}
