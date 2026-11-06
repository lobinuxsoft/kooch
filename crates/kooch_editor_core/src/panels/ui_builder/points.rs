//! Handles for a material's `@point` parameters and a node's `-kooch-curve`: a curve is drawn by
//! dragging, not by typing.

use kooch_render::material::ShaderParam;
use kooch_ui::UiPath;
use kooch_ui::style::{InlineStyle, PARAM_PREFIX};

use crate::ui_builder::UiBuilder;

const RADIUS: f32 = 5.0;
const CURVE: &str = "-kooch-curve";
/// What a node starts with: a gentle arch across it.
const ARCH: [[f32; 2]; 3] = [[0.1, 0.5], [0.5, 0.1], [0.9, 0.5]];
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
    let names: Vec<String> = (1..=path.points.len())
        .map(|n| format!("Point {n}"))
        .collect();
    let edit = handles(ui, origin, screen, &path.points, &names, path.removable());
    match (edit, added) {
        (Some(Edit::Moved(index, point)), _) => path.points[index] = point,
        (Some(Edit::Removed(index)), _) => {
            path.points.remove(index);
        }
        (None, Some((index, point))) => path.points.insert(index, point),
        (None, None) => return,
    }
    write(builder, origin, CURVE, &path.written());
}

fn on_screen(screen: egui::Rect, [x, y]: [f32; 2]) -> egui::Pos2 {
    screen.min + screen.size() * egui::vec2(x, y)
}

/// Where `pointer` sits across the element, kept inside it.
fn across(screen: egui::Rect, pointer: egui::Pos2) -> [f32; 2] {
    let at = (pointer - screen.min) / screen.size();
    [at.x.clamp(0.0, 1.0), at.y.clamp(0.0, 1.0)]
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

/// A row per point and what the path as a whole does, or a button to add it: `none` is the
/// engine's default.
pub(super) fn field(ui: &mut egui::Ui, value: Option<&str>) -> Option<String> {
    let parsed = value.and_then(|value| UiPath::parse(value).ok().flatten());
    let Some(mut path) = parsed else {
        let add = super::fields::fill(ui, egui::Button::new("Add curve"))
            .on_hover_text("None: children stay straight");
        let arch = UiPath {
            points: ARCH.to_vec(),
            closed: false,
        };
        return add.clicked().then(|| arch.written());
    };
    let mut changed = false;
    let mut removed = None;
    let removable = path.removable();
    ui.vertical(|ui| {
        let height = ui.spacing().interact_size.y;
        for (index, point) in path.points.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                let name = egui::Label::new((index + 1).to_string());
                ui.add_sized([POINT_NAME, height], name);
                let gap = ui.spacing().item_spacing.x;
                let width = ui.available_width() - REMOVE - gap * 2.0;
                changed |= super::fields::axes(ui, point, width, 0.0..=1.0);
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
                && let Some(&[x, y]) = path.points.last()
            {
                path.points.push([(x + 0.1).min(1.0), y]);
                changed = true;
            }
        });
    });
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
