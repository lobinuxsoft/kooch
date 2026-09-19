//! The debug controls section: view modes, LOD, lights and the switches behind them.

use super::*;

pub(super) fn debug_controls(
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
    // What the isolated light actually casts (#743). A point light with no shadow renders exactly
    // like one whose shadow broke, and the view has nothing to draw that would tell them apart — so
    // the limitation is written down instead of left to be inferred.
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
    // 🔴 The same problem the note above solves, for the two views that shipped painting the whole
    // screen one colour: a code the reader has to remember is a code the reader does not have.
    // Orange means "pick a lamp", and saying so is one line.
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
    // The scale is a control, not a caption. A heatmap's top of scale is the one number that
    // decides whether the picture says anything: at 16 a hundred-light stress scene is flat red and
    // at 40 the same frame separates into froxels.
    if *meshlet_debug_mode == MeshletDebugMode::LightsPerPixel {
        // 🔴 The measurement, rather than a colour to squint at.
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
    if *meshlet_debug_mode == MeshletDebugMode::LightsPerPixel {
        // 🔴 Both ends, because the window is what matters and the near one is the stronger lever:
        // 24 slices spread over [5, 200] put a 5.1 m froxel at 30 m, and over [20, 60] put a 1.4 m
        // one there.
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
    // a light contributing a fraction of the frame's exposure spends all of it on a highlight
    // nobody can see. Zero is off, and off is what every frame did before this existed.
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
    // 🔴 A READOUT now, for the same reason as the shadow-page section below.
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("LOD ≤").small());
        ui.label(
            egui::RichText::new(format!(
                "{:.2} px",
                meshlet_lod_settings.target_error_pixels
            ))
            .small()
            .strong(),
        );
        ui.label(egui::RichText::new("· render settings").small().weak())
            .on_hover_text(
                "Pixel-error threshold for the continuous-LOD selector, from the \
                 project's render settings (`meshlet_lod_error`, group Geometry). \
                 Lower keeps more meshlets at any given distance; raising it walks \
                 every object down its chain at once. Edited there rather than here \
                 so a game gets the value the project chose — as a panel knob it \
                 reached the editor and nothing else.",
            );
    });
}
