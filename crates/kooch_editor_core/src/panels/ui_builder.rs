//! The UI Builder panel: a toolbar and the canvas. The middle button pans, the wheel zooms around
//! the cursor one step at a time and F frames the canvas, as Unity's UI Builder does.

use crate::ui_builder::{RESOLUTIONS, UiBuilder};

/// Picked from the toolbar.
const MENU_ZOOMS: [f32; 12] = [
    0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 2.5, 4.0, 5.0, 10.0, 25.0,
];
const MIN_ZOOM: f32 = 0.25;
const MAX_ZOOM: f32 = 100.0;
/// Points around a framed canvas.
const FRAME_MARGIN: f32 = 20.0;
const CHECKER: f32 = 16.0;
/// Largest side the canvas renders at: past it a deep zoom magnifies instead of re-rendering.
const MAX_SIDE: f32 = 4096.0;

pub(crate) fn draw_ui_builder(ui: &mut egui::Ui, builder: Option<&mut UiBuilder>) {
    let Some(builder) = builder else {
        ui.centered_and_justified(|ui| {
            ui.label("Double-click a .kxml in the Asset Browser to open it here.");
        });
        return;
    };
    toolbar(ui, builder);
    ui.separator();
    canvas(ui, builder);
}

fn toolbar(ui: &mut egui::Ui, builder: &mut UiBuilder) {
    ui.horizontal(|ui| {
        let name = builder
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        ui.strong(name);
        ui.separator();

        let current = RESOLUTIONS
            .iter()
            .find(|(w, h, _)| (*w, *h) == builder.resolution)
            .map_or_else(
                || format!("{}×{}", builder.resolution.0, builder.resolution.1),
                |(_, _, label)| (*label).to_owned(),
            );
        egui::ComboBox::from_id_salt("ui_builder_resolution")
            .selected_text(current)
            .show_ui(ui, |ui| {
                for &(w, h, label) in RESOLUTIONS {
                    if ui
                        .selectable_label(builder.resolution == (w, h), label)
                        .clicked()
                    {
                        builder.resolution = (w, h);
                        builder.pan = None;
                    }
                }
            })
            .response
            .on_hover_text("The screen the canvas is laid out for");

        let sheet = builder
            .style
            .and_then(|guid| builder.sheets.iter().find(|(own, _)| *own == guid))
            .map_or("Default sheet".to_owned(), |(_, name)| name.clone());
        let mut picked = None;
        egui::ComboBox::from_id_salt("ui_builder_sheet")
            .selected_text(sheet)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(builder.style.is_none(), "Default sheet")
                    .clicked()
                {
                    picked = Some(None);
                }
                for (guid, name) in &builder.sheets {
                    if ui
                        .selectable_label(builder.style == Some(*guid), name)
                        .clicked()
                    {
                        picked = Some(Some(*guid));
                    }
                }
            })
            .response
            .on_hover_text("The .kcss the canvas wears; the markup never names one");
        if let Some(style) = picked {
            builder.set_style(style);
        }

        egui::ComboBox::from_id_salt("ui_builder_zoom")
            .selected_text(format!("{:.0}%", builder.zoom * 100.0))
            .width(70.0)
            .show_ui(ui, |ui| {
                for zoom in MENU_ZOOMS {
                    if ui
                        .selectable_label(
                            (builder.zoom - zoom).abs() < 1e-3,
                            format!("{:.0}%", zoom * 100.0),
                        )
                        .clicked()
                    {
                        builder.pending_zoom = Some(zoom);
                    }
                }
            });
        if ui
            .button("Fit")
            .on_hover_text("Frame the whole canvas (F)")
            .clicked()
        {
            builder.pan = None;
        }
    });
}

fn canvas(ui: &mut egui::Ui, builder: &mut UiBuilder) {
    let (rect, response) =
        ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
    let ppp = ui.ctx().pixels_per_point();
    let resolution = egui::vec2(builder.resolution.0 as f32, builder.resolution.1 as f32);
    if builder.pan.is_none() {
        frame(builder, rect, resolution, ppp);
    }
    if let Some(zoom) = builder.pending_zoom.take() {
        zoom_at(builder, rect, rect.center(), zoom);
    }
    if response.hovered() {
        // The wheel's own events, not the smoothed delta: one notch is one step.
        let scroll: f32 = ui.input(|input| {
            input
                .events
                .iter()
                .filter_map(|event| match event {
                    egui::Event::MouseWheel { delta, .. } => Some(delta.y),
                    _ => None,
                })
                .sum()
        });
        if scroll != 0.0
            && let Some(cursor) = response.hover_pos()
        {
            zoom_at(builder, rect, cursor, step(builder.zoom, scroll > 0.0));
        }
        if ui.input(|input| input.key_pressed(egui::Key::F)) {
            frame(builder, rect, resolution, ppp);
        }
    }
    let pan = builder.pan.get_or_insert(egui::Vec2::ZERO);
    if response.dragged_by(egui::PointerButton::Middle) {
        *pan += response.drag_delta();
    }
    let canvas = egui::Rect::from_min_size(rect.min + *pan, resolution * builder.zoom / ppp);

    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);
    checker(&painter, canvas, canvas.intersect(rect));
    if let Some(texture) = builder.texture {
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        painter.image(texture, canvas, uv, egui::Color32::WHITE);
    }
    painter.rect_stroke(
        canvas,
        0.0,
        egui::Stroke::new(1.0, ui.visuals().weak_text_color()),
        egui::StrokeKind::Outside,
    );

    // Rendered at the size it is shown, so text stays sharp at any zoom, up to `MAX_SIDE`.
    let pixels = resolution * builder.zoom;
    let fit = (MAX_SIDE / pixels.max_elem()).min(1.0);
    builder.request = Some((
        (pixels.x * fit).round().max(1.0) as u32,
        (pixels.y * fit).round().max(1.0) as u32,
    ));
}

/// Fits the canvas in the panel with a margin, centred. Not snapped to a zoom step.
fn frame(builder: &mut UiBuilder, rect: egui::Rect, resolution: egui::Vec2, ppp: f32) {
    let room = (rect.size() - egui::Vec2::splat(FRAME_MARGIN * 2.0)).max(egui::Vec2::splat(1.0));
    let points = resolution / ppp;
    builder.zoom = (room.x / points.x)
        .min(room.y / points.y)
        .clamp(MIN_ZOOM, MAX_ZOOM);
    builder.pan = Some((rect.size() - points * builder.zoom) / 2.0);
}

/// Changes the zoom keeping the canvas point under `at` where it is.
fn zoom_at(builder: &mut UiBuilder, rect: egui::Rect, at: egui::Pos2, zoom: f32) {
    let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
    let origin = rect.min + builder.pan.unwrap_or_default();
    let under = (at - origin) / builder.zoom;
    builder.pan = Some(at - under * zoom - rect.min);
    builder.zoom = zoom;
}

/// Unity's steps: 5% to 145%, 25% to 475%, then 250% to 10000%.
fn steps() -> impl Iterator<Item = f32> {
    (25..=145)
        .step_by(5)
        .chain((150..=475).step_by(25))
        .chain((500..=10000).step_by(250))
        .map(|percent| percent as f32 / 100.0)
}

fn step(zoom: f32, up: bool) -> f32 {
    match up {
        true => steps().find(|&s| s > zoom + 1e-3).unwrap_or(MAX_ZOOM),
        false => steps()
            .filter(|&s| s < zoom - 1e-3)
            .last()
            .unwrap_or(MIN_ZOOM),
    }
}

/// Transparency behind the canvas, in tiles of a fixed screen size, only where it shows.
fn checker(painter: &egui::Painter, canvas: egui::Rect, visible: egui::Rect) {
    if !(visible.width() > 0.0 && visible.height() > 0.0) {
        return;
    }
    painter.rect_filled(visible, 0.0, egui::Color32::from_gray(60));
    let first = ((visible.min - canvas.min) / CHECKER).floor();
    let last = ((visible.max - canvas.min) / CHECKER).ceil();
    for row in first.y as i32..last.y as i32 {
        for column in first.x as i32..last.x as i32 {
            if (row + column).rem_euclid(2) == 0 {
                continue;
            }
            let min = canvas.min + egui::vec2(column as f32, row as f32) * CHECKER;
            let tile =
                egui::Rect::from_min_size(min, egui::Vec2::splat(CHECKER)).intersect(visible);
            painter.rect_filled(tile, 0.0, egui::Color32::from_gray(80));
        }
    }
}
