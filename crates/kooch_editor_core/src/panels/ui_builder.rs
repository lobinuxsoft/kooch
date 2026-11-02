//! The UI Builder panel: a toolbar, the hierarchy, the canvas and the Inspector. The middle button
//! pans, the wheel zooms around the cursor one step at a time and F frames the canvas, as Unity's
//! UI Builder does.

use crate::panels::inspector::AssetCatalogEntry;
use crate::ui_builder::SHEET_TYPE;

mod actions;
mod classes;
mod hierarchy;
mod inspector;
mod styles;

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
/// Shares of the panel the hierarchy and the Inspector take beside the canvas.
const HIERARCHY_SHARE: f32 = 0.18;
const INSPECTOR_SHARE: f32 = 0.24;

pub(crate) fn draw_ui_builder(
    ui: &mut egui::Ui,
    builder: Option<&mut UiBuilder>,
    catalog: &[AssetCatalogEntry],
) {
    let Some(builder) = builder else {
        ui.centered_and_justified(|ui| {
            ui.label("Double-click a .kxml in the Asset Browser to open it here.");
        });
        return;
    };
    toolbar(ui, builder, catalog);
    ui.separator();
    let full = ui.available_rect_before_wrap();
    let left = full.with_max_x(full.min.x + full.width() * HIERARCHY_SHARE);
    let right = full.with_min_x(full.max.x - full.width() * INSPECTOR_SHARE);
    let middle = full.with_min_x(left.max.x).with_max_x(right.min.x);
    ui.scope_builder(egui::UiBuilder::new().max_rect(left.shrink(4.0)), |ui| {
        hierarchy::draw(ui, builder);
    });
    ui.scope_builder(egui::UiBuilder::new().max_rect(middle), |ui| {
        canvas(ui, builder);
    });
    ui.scope_builder(egui::UiBuilder::new().max_rect(right.shrink(4.0)), |ui| {
        inspector::draw(ui, builder, catalog);
    });
    ui.advance_cursor_after_rect(full);
}

fn toolbar(ui: &mut egui::Ui, builder: &mut UiBuilder, catalog: &[AssetCatalogEntry]) {
    ui.horizontal(|ui| {
        let name = builder
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        ui.strong(match builder.dirty {
            true => format!("{name} *"),
            false => name,
        });
        if ui
            .add_enabled(builder.dirty, egui::Button::new("Save"))
            .on_hover_text("Write the edits to the .kxml")
            .clicked()
        {
            builder.save();
        }
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

        // The .kcss the canvas wears; (None) is the default sheet.
        let picked = crate::widgets::asset_picker(
            ui,
            "ui_builder_sheet",
            builder.style,
            SHEET_TYPE,
            catalog,
            None,
        );
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
    // Canvas units per point: the layout is 1080 units tall whatever the resolution.
    let units = kooch_ui::CANVAS_HEIGHT / canvas.height().max(1.0);
    let to_screen = |rect: kooch_ui::UiRect| {
        egui::Rect::from_min_size(
            canvas.min + egui::vec2(rect.min.x, rect.min.y) / units,
            egui::vec2(rect.size.x, rect.size.y) / units,
        )
    };
    let under = response.hover_pos().and_then(|pos| {
        let point = (pos - canvas.min) * units;
        builder.hit(glam::Vec2::new(point.x, point.y))
    });
    if response.clicked() {
        builder.selected = under;
    }

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
    let accent = ui.visuals().selection.stroke.color;
    if let Some(rect) = under.and_then(|origin| builder.rect_of(origin)) {
        let stroke = egui::Stroke::new(1.0, accent.gamma_multiply(0.5));
        painter.rect_stroke(to_screen(rect), 0.0, stroke, egui::StrokeKind::Outside);
    }
    if let Some(rect) = builder.selected.and_then(|origin| builder.rect_of(origin)) {
        let stroke = egui::Stroke::new(2.0, accent);
        painter.rect_stroke(to_screen(rect), 0.0, stroke, egui::StrokeKind::Outside);
    }
    if let Some(error) = &builder.error {
        painter.text(
            rect.left_top() + egui::vec2(8.0, 8.0),
            egui::Align2::LEFT_TOP,
            error,
            egui::FontId::proportional(13.0),
            ui.visuals().error_fg_color,
        );
    }

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
