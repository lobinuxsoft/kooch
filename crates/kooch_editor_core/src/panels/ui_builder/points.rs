//! Handles for a material's `@point` parameters and a node's `-kooch-curve`: a curve is drawn by
//! dragging, not by typing.

use glam::Vec3;
use kooch_ecs::spline::{Knot, TANGENT_CHOICES};
use kooch_render::material::ShaderParam;
use kooch_ui::style::{InlineStyle, PARAM_PREFIX};
use kooch_ui::{FLIP_X, FLIP_Y, UiPath};

use crate::ui_builder::UiBuilder;

const RADIUS: f32 = 5.0;
const CURVE: &str = "-kooch-curve";
/// What a path can start from, generated once and edited from there.
const SHAPES: [(&str, fn() -> UiPath); 4] = [
    ("Line", UiPath::segment),
    ("Arc", UiPath::arc),
    ("Circle", UiPath::circle),
    ("Square", UiPath::square),
];
/// A point's mode in the Inspector.
const MODE: f32 = 72.0;
/// A tangent's grip, smaller than a point's so the two read apart.
const GRIP: f32 = 3.5;
/// A point's number in the Inspector.
const POINT_NAME: f32 = 24.0;
/// Its remove button.
const REMOVE: f32 = 22.0;
/// How far from the path a double-click still lands on it.
const REACH: f32 = 8.0;

/// What a handle did this frame.
enum Edit {
    Moved(usize, [f32; 2]),
    Removed(usize),
}

/// `screen` with the corners a flip swaps, so a fraction across it lands where the node draws it.
pub(super) fn turned(screen: egui::Rect, flip: u32) -> egui::Rect {
    let mut turned = screen;
    if flip & FLIP_X != 0 {
        std::mem::swap(&mut turned.min.x, &mut turned.max.x);
    }
    if flip & FLIP_Y != 0 {
        std::mem::swap(&mut turned.min.y, &mut turned.max.y);
    }
    turned
}

/// `screen` is the element's rect in points; a point at `[0, 0]` sits on its top left corner.
pub(super) fn draw(ui: &mut egui::Ui, builder: &mut UiBuilder, origin: u32, screen: egui::Rect) {
    draw_params(ui, builder, origin, screen);
    draw_curve(ui, builder, origin, screen);
}

fn draw_params(ui: &mut egui::Ui, builder: &mut UiBuilder, origin: u32, screen: egui::Rect) {
    let params: Vec<ShaderParam> = builder
        .params_of(origin)
        .iter()
        .filter(|param| param.point)
        .cloned()
        .collect();
    if params.is_empty() {
        return;
    }
    let points: Vec<[f32; 2]> = params
        .iter()
        .map(|param| current(builder, origin, param))
        .collect();
    let names: Vec<String> = params.iter().map(|param| param.name.clone()).collect();
    let at = |&[x, y]: &[f32; 2]| on_screen(screen, [x, y]);
    let polygon: Vec<egui::Pos2> = points.iter().map(at).collect();
    let stroke = egui::Stroke::new(1.0, ui.visuals().selection.stroke.color);
    // In declaration order, which for a curve is the order of its control polygon.
    ui.painter()
        .add(egui::Shape::dashed_line(&polygon, stroke, 4.0, 3.0));
    let edit = handles(ui, origin, screen, &points, &names, false);
    let Some(Edit::Moved(index, [x, y])) = edit else {
        return;
    };
    let property = format!("{PARAM_PREFIX}{}", params[index].name.to_ascii_lowercase());
    write(builder, origin, &property, &format!("{x:.3} {y:.3}"));
}

fn draw_curve(ui: &mut egui::Ui, builder: &mut UiBuilder, origin: u32, screen: egui::Rect) {
    let resolved = builder.known.styles.get(origin as usize);
    let Some(mut path) = resolved.and_then(|style| style.curve.clone()) else {
        return;
    };
    let added = line(ui, origin, screen, &path);
    let steered = grips(ui, origin, screen, &path);
    let names: Vec<String> = (1..=path.points.len())
        .map(|n| format!("Point {n}"))
        .collect();
    let places: Vec<[f32; 2]> = path.points.iter().map(flat).collect();
    let edit = handles(ui, origin, screen, &places, &names, path.removable());
    match (edit, steered, added) {
        (Some(Edit::Moved(index, [x, y])), ..) => {
            path.points[index].position = Vec3::new(x, y, 0.0);
        }
        (Some(Edit::Removed(index)), ..) => {
            path.points.remove(index);
        }
        (None, Some((index, arriving, to)), _) => path.steer(index, arriving, to),
        (None, None, Some((index, [x, y]))) => {
            path.points.insert(index, Knot::at(Vec3::new(x, y, 0.0)));
        }
        (None, None, None) => return,
    }
    write(builder, origin, CURVE, &path.written());
}

fn flat(knot: &Knot) -> [f32; 2] {
    [knot.position.x, knot.position.y]
}

fn on_screen(screen: egui::Rect, [x, y]: [f32; 2]) -> egui::Pos2 {
    screen.min + screen.size() * egui::vec2(x, y)
}

/// Where `pointer` sits across the element, kept inside it.
fn across(screen: egui::Rect, pointer: egui::Pos2) -> [f32; 2] {
    outside(screen, pointer).map(|at| at.clamp(0.0, 1.0))
}

/// Where `pointer` sits across the element, past its edges too: a tangent may reach beyond the box.
fn outside(screen: egui::Rect, pointer: egui::Pos2) -> [f32; 2] {
    let at = (pointer - screen.min) / screen.size();
    [at.x, at.y]
}

/// The path itself; a double-click on it gives the index a new point goes in, and the point.
fn line(
    ui: &mut egui::Ui,
    origin: u32,
    screen: egui::Rect,
    path: &UiPath,
) -> Option<(usize, [f32; 2])> {
    let samples = path.sampled();
    let points: Vec<egui::Pos2> = samples
        .iter()
        .map(|(_, at)| on_screen(screen, *at))
        .collect();
    let stroke = egui::Stroke::new(1.0, ui.visuals().selection.stroke.color);
    ui.painter().add(egui::Shape::line(points.clone(), stroke));
    let mut added = None;
    for (index, (&(stretch, _), &pos)) in samples.iter().zip(&points).enumerate() {
        let area = egui::Rect::from_center_size(pos, egui::Vec2::splat(REACH * 2.0));
        let id = ui.id().with(("ui_builder_path", origin, index));
        let response = ui
            .interact(area, id, egui::Sense::click())
            .on_hover_text("Double-click adds a point here");
        if response.double_clicked()
            && path.growable()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            added = Some((stretch + 1, across(screen, pointer)));
        }
    }
    added
}

/// A grip per tangent on a line from its point; the one being dragged, whether it is the arriving
/// one, and where to.
fn grips(
    ui: &mut egui::Ui,
    origin: u32,
    screen: egui::Rect,
    path: &UiPath,
) -> Option<(usize, bool, [f32; 2])> {
    let accent = ui.visuals().selection.stroke.color;
    let painter = ui.painter().clone();
    let mut dragged = None;
    for (index, knot) in path.points.iter().enumerate() {
        let from = on_screen(screen, flat(knot));
        for (arriving, grip) in [true, false].into_iter().zip(path.grips(index)) {
            let Some(grip) = grip else {
                continue;
            };
            let pos = on_screen(screen, grip);
            painter.line_segment(
                [from, pos],
                egui::Stroke::new(1.0, accent.gamma_multiply(0.6)),
            );
            painter.circle_filled(pos, GRIP, accent);
            let square = egui::Rect::from_center_size(pos, egui::Vec2::splat(GRIP * 4.0));
            let id = ui.id().with(("ui_builder_grip", origin, index, arriving));
            let response = ui
                .interact(square, id, egui::Sense::drag())
                .on_hover_text("Drag to steer the curve through this point")
                .on_hover_cursor(egui::CursorIcon::Grab);
            if response.dragged_by(egui::PointerButton::Primary)
                && let Some(pointer) = response.interact_pointer_pos()
            {
                dragged = Some((index, arriving, outside(screen, pointer)));
            }
        }
    }
    dragged
}

/// `points` as handles, each named; the one being dragged or, when `removable`, right-clicked.
fn handles(
    ui: &mut egui::Ui,
    origin: u32,
    screen: egui::Rect,
    points: &[[f32; 2]],
    names: &[String],
    removable: bool,
) -> Option<Edit> {
    let accent = ui.visuals().selection.stroke.color;
    let painter = ui.painter().clone();
    let mut edit = None;
    for (index, (name, point)) in names.iter().zip(points).enumerate() {
        let pos = on_screen(screen, *point);
        let square = egui::Rect::from_center_size(pos, egui::Vec2::splat(RADIUS * 3.0));
        let id = ui.id().with(("ui_builder_point", origin, name));
        let hint = match removable {
            true => format!("{name}: right-click removes it"),
            false => name.clone(),
        };
        let response = ui
            .interact(square, id, egui::Sense::click_and_drag())
            .on_hover_text(hint)
            .on_hover_cursor(egui::CursorIcon::Grab);
        painter.circle(
            pos,
            RADIUS,
            ui.visuals().extreme_bg_color,
            egui::Stroke::new(1.5, accent),
        );
        if response.dragged_by(egui::PointerButton::Primary)
            && let Some(pointer) = response.interact_pointer_pos()
        {
            edit = Some(Edit::Moved(index, across(screen, pointer)));
        }
        if removable && response.secondary_clicked() {
            edit = Some(Edit::Removed(index));
        }
    }
    edit
}

fn write(builder: &mut UiBuilder, origin: u32, property: &str, value: &str) {
    let Some(written) = builder.inspect(origin).map(|inspected| inspected.style) else {
        return;
    };
    let mut style = InlineStyle::parse(&written);
    style.set(property, Some(value));
    builder.set_attribute(origin, "style", &style.write());
}

/// The shape picked from a list reading `label`, if one was.
fn shape(ui: &mut egui::Ui, label: &str, hint: &str) -> Option<UiPath> {
    let mut picked = None;
    super::fields::wide(ui, |ui| {
        egui::ComboBox::from_id_salt("ui_builder_curve_shape")
            .selected_text(label)
            .show_ui(ui, |ui| {
                for (name, made) in SHAPES {
                    if ui.selectable_label(false, name).clicked() {
                        picked = Some(made());
                    }
                }
            })
            .response
            .on_hover_text(hint);
    });
    picked
}

/// Its mode from a list; whether it changed.
fn mode(ui: &mut egui::Ui, index: usize, path: &mut UiPath) -> bool {
    let held = path.points[index].mode;
    let name = TANGENT_CHOICES.iter().find(|c| c.value == i64::from(held));
    let mut picked = None;
    egui::ComboBox::from_id_salt(("ui_builder_curve_mode", index))
        .selected_text(name.map_or("?", |choice| choice.label))
        .width(MODE)
        .show_ui(ui, |ui| {
            for choice in TANGENT_CHOICES {
                let on = choice.value == i64::from(held);
                if ui.selectable_label(on, choice.label).clicked() {
                    picked = u32::try_from(choice.value).ok();
                }
            }
        })
        .response
        .on_hover_text("Auto follows its neighbours, Aligned keeps a smooth handle pair, Broken lets the two make a corner");
    if let Some(picked) = picked.filter(|picked| *picked != held) {
        path.set_mode(index, picked);
        return true;
    }
    false
}

/// A row per point and what the path as a whole does, or a shape to start from: `none` is the
/// engine's default.
pub(super) fn field(ui: &mut egui::Ui, value: Option<&str>) -> Option<String> {
    let parsed = value.and_then(|value| UiPath::parse(value).ok().flatten());
    let Some(mut path) = parsed else {
        let hint = "None: children stay straight";
        return shape(ui, "Add curve", hint).map(|path| path.written());
    };
    let mut changed = false;
    let mut removed = None;
    let mut replaced = None;
    let removable = path.removable();
    ui.vertical(|ui| {
        let height = ui.spacing().interact_size.y;
        for index in 0..path.points.len() {
            ui.horizontal(|ui| {
                let name = egui::Label::new((index + 1).to_string());
                ui.add_sized([POINT_NAME, height], name);
                let gap = ui.spacing().item_spacing.x;
                let width = ui.available_width() - MODE - REMOVE - gap * 4.0;
                let mut place = flat(&path.points[index]);
                if super::fields::axes(ui, &mut place, width, 0.0..=1.0) {
                    path.points[index].position = Vec3::new(place[0], place[1], 0.0);
                    changed = true;
                }
                changed |= mode(ui, index, &mut path);
                let remove = egui::Button::new(crate::icons::X);
                let response = ui
                    .add_enabled_ui(removable, |ui| ui.add_sized([REMOVE, height], remove))
                    .inner
                    .on_hover_text("Removes this point")
                    .on_disabled_hover_text("A path needs the points it has");
                if response.clicked() {
                    removed = Some(index);
                }
            });
        }
        ui.horizontal(|ui| {
            changed |= ui
                .checkbox(&mut path.closed, "Closed")
                .on_hover_text("Joins the last point to the first, as a ring")
                .changed();
            let add = egui::Button::new("Add point");
            let growable = path.growable();
            let response = ui
                .add_enabled_ui(growable, |ui| super::fields::fill(ui, add))
                .inner
                .on_hover_text("Adds one after the last; dragging it places it")
                .on_disabled_hover_text("A path takes no more points");
            if response.clicked()
                && let Some(last) = path.points.last()
            {
                let [x, y] = flat(last);
                path.points
                    .push(Knot::at(Vec3::new((x + 0.1).min(1.0), y, 0.0)));
                changed = true;
            }
        });
        let hint = "Replaces this path, points and handles, with the shape";
        replaced = shape(ui, "Replace with shape", hint);
    });
    if let Some(picked) = replaced {
        return Some(picked.written());
    }
    if let Some(index) = removed {
        path.points.remove(index);
        changed = true;
    }
    // A ring of two is a line walked twice: it opens instead of being refused on reading back.
    if path.points.len() < 3 {
        path.closed = false;
    }
    changed.then(|| path.written())
}

/// What the element gets for `param`: its sheet and inline style, or the shader's default.
fn current(builder: &UiBuilder, origin: u32, param: &ShaderParam) -> [f32; 2] {
    let styled = builder.known.styles.get(origin as usize).and_then(|style| {
        let name = param.name.to_ascii_lowercase();
        style.params.iter().find(|(own, _)| **own == *name)
    });
    let [x, y, ..] = styled.map_or(param.default, |(_, value)| *value);
    [x, y]
}
