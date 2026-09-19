//! Groups and notes, drawn behind the nodes and handled in front of them (#1211).
//!
//! 🔴 Painted into a slot reserved before the graph widget draws, so they sit under its nodes; the
//! widget's own background is made transparent and its fill painted here first, or it would cover
//! them. Their handles live on a sublayer registered after the widget's: the widget takes the
//! pointer on a sublayer of its own, which sits above anything on the panel's layer.

use egui::emath::TSTransform;
use egui::layers::ShapeIdx;
use std::collections::HashMap;

use egui::{Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, Vec2};

use crate::shader_graph::Graph;
use crate::shader_graph::annotations::{self, Annotations, GROUP_COLORS};

/// Height of a group's title bar, in graph units.
pub(super) const HEADER: f32 = 28.0;

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
    snarl_id: egui::Id,
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

    annotations.prune(graph);
    let rects: HashMap<usize, Rect> = egui_snarl::ui::get_node_rects(snarl_id, ui.ctx())
        .into_iter()
        .map(|(id, rect)| (id.0, rect))
        .collect();
    let frames: Vec<Option<Rect>> = annotations
        .groups
        .iter()
        .map(|group| annotations::fit(group, graph, &rects, HEADER))
        .collect();
    drop_into_groups(ui, annotations, graph, &frames, &rects);

    let mut ungrouped = None;
    for (index, frame) in frames.iter().enumerate() {
        let (Some(frame), Some(group)) = (frame, annotations.groups.get(index)) else {
            continue;
        };
        let screen = to_screen * *frame;
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
        let renaming = renaming(ui) == Some(index);
        if !renaming {
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
        }

        let moved = ui
            .interact(
                header,
                ui.id().with(("graph_group", index)),
                Sense::click_and_drag(),
            )
            .on_hover_cursor(egui::CursorIcon::Grab)
            .on_hover_text("Double-click to rename · drag to move · right-click for more");
        if moved.double_clicked() {
            set_renaming(ui, Some(index));
        }
        if renaming {
            rename_in_place(ui, header, &mut annotations.groups[index].title);
        }
        if moved.dragged() {
            annotations::move_group(annotations, graph, index, moved.drag_delta() / scale);
            // A group carried over a node must not look like that node being dropped into it.
            ui.ctx().data_mut(|d| d.insert_temp(group_drag_id(), true));
        }
        editing_menu(&moved, |ui| {
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
                .button("Ungroup")
                .on_hover_text("Remove the frame; its nodes stay where they are")
                .clicked()
            {
                ungrouped = Some(index);
                ui.close();
            }
        });
    }
    if let Some(index) = ungrouped {
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
            .interact(
                screen,
                ui.id().with(("graph_note", index)),
                Sense::click_and_drag(),
            )
            .on_hover_cursor(egui::CursorIcon::Grab);
        if body.dragged() {
            annotations.notes[index].pos += body.drag_delta() / scale;
        }
        editing_menu(&body, |ui| {
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

/// On the frame a node drag ends, the nodes it moved join the group they were dropped in — by their
/// centre, and only when they are not in it already. Where the drag began is kept from the press.
fn drop_into_groups(
    ui: &egui::Ui,
    annotations: &mut Annotations,
    graph: &Graph,
    frames: &[Option<Rect>],
    rects: &HashMap<usize, Rect>,
) {
    let ctx = ui.ctx();
    let (pressed, released) =
        ctx.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_released()));
    if pressed {
        let start: Vec<(usize, Pos2)> = graph
            .nodes_pos_ids()
            .map(|(id, pos, _)| (id.0, pos))
            .collect();
        ctx.data_mut(|d| {
            d.insert_temp(drag_start_id(), start);
            d.insert_temp(group_drag_id(), false);
        });
    }
    if !released {
        return;
    }
    let start = ctx.data(|d| d.get_temp::<Vec<(usize, Pos2)>>(drag_start_id()));
    let carried = ctx.data(|d| d.get_temp::<bool>(group_drag_id()).unwrap_or(false));
    let Some(start) = start.filter(|_| !carried) else {
        return;
    };
    for (node, before) in start {
        let Some(pos) = graph
            .get_node_info(egui_snarl::NodeId(node))
            .map(|info| info.pos)
        else {
            continue;
        };
        if pos == before {
            continue;
        }
        let centre = rects
            .get(&node)
            .map_or(pos + crate::shader_graph::NODE_SIZE / 2.0, |rect| {
                rect.center()
            });
        let target = frames.iter().enumerate().find(|(index, frame)| {
            frame.is_some_and(|frame| frame.contains(centre))
                && !annotations.groups[*index].members.contains(&node)
        });
        if let Some((index, _)) = target {
            annotations.join(index, &[node]);
        }
    }
}

/// A right-click menu that stays open while its fields are clicked into. egui's default closes on
/// any click, inside included, which shut the menu before a title could be typed.
fn editing_menu(response: &egui::Response, contents: impl FnOnce(&mut egui::Ui)) {
    egui::Popup::context_menu(response)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(contents);
}

/// The group whose title is being edited in place, if any.
fn renaming(ui: &egui::Ui) -> Option<usize> {
    ui.ctx()
        .data(|d| d.get_temp::<Option<usize>>(rename_id()))
        .flatten()
}

fn set_renaming(ui: &egui::Ui, index: Option<usize>) {
    ui.ctx().data_mut(|d| d.insert_temp(rename_id(), index));
}

fn rename_id() -> egui::Id {
    egui::Id::new("shader_graph_group_rename")
}

/// A text field over the title bar. Enter, Escape or a click elsewhere ends it.
fn rename_in_place(ui: &mut egui::Ui, header: Rect, title: &mut String) {
    let field = ui.put(
        header.shrink2(Vec2::new(4.0, 2.0)),
        egui::TextEdit::singleline(title).font(egui::TextStyle::Body),
    );
    if !field.has_focus() && !field.lost_focus() {
        field.request_focus();
    }
    if field.lost_focus() {
        set_renaming(ui, None);
    }
}

fn drag_start_id() -> egui::Id {
    egui::Id::new("shader_graph_drag_start")
}

fn group_drag_id() -> egui::Id {
    egui::Id::new("shader_graph_group_drag")
}
