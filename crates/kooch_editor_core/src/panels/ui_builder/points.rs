//! Handles for a material's `@point` parameters and a node's `-kooch-curve`: a curve is drawn by
//! dragging, not by typing.

use kooch_render::material::ShaderParam;
use kooch_ui::style::{InlineStyle, PARAM_PREFIX};

use crate::ui_builder::UiBuilder;

const RADIUS: f32 = 5.0;
const CURVE: &str = "-kooch-curve";
/// What a node starts with: a gentle arch across it.
const ARCH: [[f32; 2]; 3] = [[0.1, 0.5], [0.5, 0.1], [0.9, 0.5]];
const NAMES: [&str; 3] = ["start", "control", "end"];
/// `control` and its gap.
const POINT_NAME: f32 = 56.0;

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
    let names: Vec<&str> = params.iter().map(|param| param.name.as_str()).collect();
    let Some((index, [x, y])) = handles(ui, origin, screen, &points, &names) else {
        return;
    };
    let property = format!("{PARAM_PREFIX}{}", params[index].name.to_ascii_lowercase());
    write(builder, origin, &property, &format!("{x:.3} {y:.3}"));
}

fn draw_curve(ui: &mut egui::Ui, builder: &mut UiBuilder, origin: u32, screen: egui::Rect) {
    let resolved = builder.known.styles.get(origin as usize);
    let Some(mut curve) = resolved.and_then(|style| style.curve) else {
        return;
    };
    if let Some((index, point)) = handles(ui, origin, screen, &curve, &NAMES) {
        curve[index] = point;
        write(builder, origin, CURVE, &written(curve));
    }
}

/// `points` as handles over a dashed polygon, in order; the one being dragged, and where to.
fn handles(
    ui: &mut egui::Ui,
    origin: u32,
    screen: egui::Rect,
    points: &[[f32; 2]],
    names: &[&str],
) -> Option<(usize, [f32; 2])> {
    let at: Vec<egui::Pos2> = points
        .iter()
        .map(|&[x, y]| screen.min + screen.size() * egui::vec2(x, y))
        .collect();
    let accent = ui.visuals().selection.stroke.color;
    let painter = ui.painter().clone();
    painter.add(egui::Shape::dashed_line(
        &at,
        egui::Stroke::new(1.0, accent),
        4.0,
        3.0,
    ));
    let mut moved = None;
    for (index, (&name, &pos)) in names.iter().zip(&at).enumerate() {
        let square = egui::Rect::from_center_size(pos, egui::Vec2::splat(RADIUS * 3.0));
        let id = ui.id().with(("ui_builder_point", origin, name));
        let response = ui
            .interact(square, id, egui::Sense::drag())
            .on_hover_text(name)
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
            let across = (pointer - screen.min) / screen.size();
            moved = Some((index, [across.x.clamp(0.0, 1.0), across.y.clamp(0.0, 1.0)]));
        }
    }
    moved
}

fn write(builder: &mut UiBuilder, origin: u32, property: &str, value: &str) {
    let Some(written) = builder.inspect(origin).map(|inspected| inspected.style) else {
        return;
    };
    let mut style = InlineStyle::parse(&written);
    style.set(property, Some(value));
    builder.set_attribute(origin, "style", &style.write());
}

fn written(curve: [[f32; 2]; 3]) -> String {
    let numbers: Vec<String> = curve.iter().flatten().map(|n| format!("{n:.3}")).collect();
    numbers.join(" ")
}

/// A drag per axis of each point, or a button to add the curve: `none` is the engine's default.
pub(super) fn field(ui: &mut egui::Ui, value: Option<&str>) -> Option<String> {
    let numbers: Vec<f32> = value
        .unwrap_or_default()
        .split_whitespace()
        .filter_map(|part| part.parse().ok())
        .collect();
    let &[x0, y0, x1, y1, x2, y2] = numbers.as_slice() else {
        let add = super::fields::fill(ui, egui::Button::new("Add curve"))
            .on_hover_text("None: children stay straight");
        return add.clicked().then(|| written(ARCH));
    };
    let mut curve = [[x0, y0], [x1, y1], [x2, y2]];
    let mut changed = false;
    ui.vertical(|ui| {
        for (name, point) in NAMES.iter().zip(&mut curve) {
            ui.horizontal(|ui| {
                let height = ui.spacing().interact_size.y;
                ui.add_sized([POINT_NAME, height], egui::Label::new(*name));
                let gap = ui.spacing().item_spacing.x;
                let width = ui.available_width() - gap;
                changed |= super::fields::axes(ui, point, width, 0.0..=1.0);
            });
        }
    });
    changed.then(|| written(curve))
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
