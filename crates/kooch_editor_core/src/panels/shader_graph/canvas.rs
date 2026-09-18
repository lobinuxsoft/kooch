//! Groups and notes, drawn behind the nodes and handled in front of them (#1211).
//!
//! 🔴 Painted into a slot reserved before the graph widget draws, so they sit under its nodes; the
//! widget's own background is made transparent and its fill painted here first, or it would cover
//! them. Their handles live on a sublayer registered after the widget's: the widget takes the
//! pointer on a sublayer of its own, which sits above anything on the panel's layer.

use egui::emath::TSTransform;
use egui::layers::ShapeIdx;
use egui::{Color32, FontId, Rect, Sense, Shape, Stroke, Vec2};

use crate::shader_graph::Graph;
use crate::shader_graph::annotations::{self, Annotations, GROUP_COLORS};

/// Height of a group's title bar, in graph units.
pub(super) const HEADER: f32 = 28.0;
/// A group is never resized smaller than this, in graph units.
const MIN_GROUP: Vec2 = Vec2::new(120.0, 80.0);
/// The corner that resizes a group, in screen pixels.
const HANDLE: f32 = 14.0;

/// Where the canvas will be painted, behind everything the graph widget adds after it.
pub(super) fn reserve(ui: &egui::Ui) -> ShapeIdx {
    ui.painter().add(Shape::Noop)
}

/// Paints the canvas into `slot` and handles dragging, resizing and each item's menu.
pub(super) fn draw(
    ui: &mut egui::Ui,
    slot: ShapeIdx,
    area: Rect,
    annotations: &mut Annotations,
    graph: &mut Graph,
    to_screen: TSTransform,
) {
    // The slot is on the panel's layer; the handles' child Ui paints on another.
    let painter = ui.painter().clone();
    let handles = egui::LayerId::new(ui.layer_id().order, ui.id().with("graph_canvas_handles"));
    ui.ctx().set_sublayer(ui.layer_id(), handles);
    let ui = &mut ui.new_child(
        egui::UiBuilder::new()
            .layer_id(handles)
            .max_rect(area)
            .sense(Sense::hover()),
    );
    let mut shapes = vec![Shape::rect_filled(
        area,
        ui.visuals().widgets.noninteractive.corner_radius,
        ui.visuals().extreme_bg_color,
    )];
    let scale = to_screen.scaling;
    let mut removed_group = None;
    for index in 0..annotations.groups.len() {
        let group = &annotations.groups[index];
        let screen = to_screen * group.rect;
        let [r, g, b] = group.color;
        let header = Rect::from_min_size(screen.min, Vec2::new(screen.width(), HEADER * scale));
        shapes.push(Shape::rect_filled(
            screen,
            6.0 * scale,
            Color32::from_rgba_unmultiplied(r, g, b, 40),
        ));
        shapes.push(Shape::rect_filled(
            header,
            6.0 * scale,
            Color32::from_rgba_unmultiplied(r, g, b, 200),
        ));
        shapes.push(Shape::rect_stroke(
            screen,
            6.0 * scale,
            Stroke::new(1.0, Color32::from_rgb(r, g, b)),
            egui::StrokeKind::Inside,
        ));
        let title = ui.painter().layout_no_wrap(
            group.title.clone(),
            FontId::proportional(14.0 * scale),
            Color32::WHITE,
        );
        shapes.push(Shape::galley(
            header.min + Vec2::new(8.0, 5.0) * scale,
            title,
            Color32::WHITE,
        ));

        let moved = ui
            .interact(header, ui.id().with(("graph_group", index)), Sense::drag())
            .on_hover_cursor(egui::CursorIcon::Grab);
        if moved.dragged() {
            annotations::move_group(annotations, graph, index, moved.drag_delta() / scale);
        }
        let corner = Rect::from_min_size(screen.max - Vec2::splat(HANDLE), Vec2::splat(HANDLE));
        let resized = ui
            .interact(
                corner,
                ui.id().with(("graph_group_size", index)),
                Sense::drag(),
            )
            .on_hover_cursor(egui::CursorIcon::ResizeNwSe);
        if resized.dragged() {
            let rect = &mut annotations.groups[index].rect;
            rect.max = (rect.max + resized.drag_delta() / scale).max(rect.min + MIN_GROUP);
        }
        moved.context_menu(|ui| {
            let group = &mut annotations.groups[index];
            ui.text_edit_singleline(&mut group.title);
            ui.horizontal(|ui| {
                for color in GROUP_COLORS {
                    let [r, g, b] = color;
                    let (swatch, response) =
                        ui.allocate_exact_size(Vec2::splat(16.0), Sense::click());
                    ui.painter()
                        .rect_filled(swatch, 3.0, Color32::from_rgb(r, g, b));
                    if response.clicked() {
                        group.color = color;
                    }
                }
            });
            if ui
                .button(format!("{} Delete group", crate::icons::TRASH))
                .clicked()
            {
                removed_group = Some(index);
                ui.close();
            }
        });
    }
    if let Some(index) = removed_group {
        annotations.groups.remove(index);
    }

    let mut removed_note = None;
    for index in 0..annotations.notes.len() {
        let note = &annotations.notes[index];
        let text = ui.painter().layout(
            note.text.clone(),
            FontId::proportional(13.0 * scale),
            Color32::from_rgb(0x20, 0x1C, 0x10),
            note.width * scale,
        );
        let at = to_screen * note.pos;
        let screen = Rect::from_min_size(at, text.size() + Vec2::splat(16.0 * scale));
        shapes.push(Shape::rect_filled(
            screen,
            4.0 * scale,
            Color32::from_rgb(0xE8, 0xD8, 0x8C),
        ));
        shapes.push(Shape::galley(
            at + Vec2::splat(8.0 * scale),
            text,
            Color32::BLACK,
        ));

        let body = ui
            .interact(screen, ui.id().with(("graph_note", index)), Sense::drag())
            .on_hover_cursor(egui::CursorIcon::Grab);
        if body.dragged() {
            annotations.notes[index].pos += body.drag_delta() / scale;
        }
        body.context_menu(|ui| {
            ui.add(
                egui::TextEdit::multiline(&mut annotations.notes[index].text)
                    .desired_width(annotations::NOTE_WIDTH),
            );
            if ui
                .button(format!("{} Delete note", crate::icons::TRASH))
                .clicked()
            {
                removed_note = Some(index);
                ui.close();
            }
        });
    }
    if let Some(index) = removed_note {
        annotations.notes.remove(index);
    }

    painter.set(slot, Shape::Vec(shapes));
}

/// A group's rect for nodes covering `bounds`: room for its title bar above them.
pub(super) fn group_around(bounds: Rect) -> Rect {
    Rect::from_min_max(bounds.min - Vec2::new(0.0, HEADER), bounds.max)
}
