//! Handles for a material's `@point` parameters: a curve is drawn by dragging, not by typing.

use kooch_render::material::ShaderParam;
use kooch_ui::style::{InlineStyle, PARAM_PREFIX};

use crate::ui_builder::UiBuilder;

const RADIUS: f32 = 5.0;

/// `screen` is the element's rect in points; a point at `[0, 0]` sits on its top left corner.
pub(super) fn draw(ui: &mut egui::Ui, builder: &mut UiBuilder, origin: u32, screen: egui::Rect) {
    let params: Vec<ShaderParam> = builder
        .params_of(origin)
        .iter()
        .filter(|param| param.point)
        .cloned()
        .collect();
    if params.is_empty() {
        return;
    }
    let at: Vec<egui::Pos2> = params
        .iter()
        .map(|param| {
            let [x, y] = current(builder, origin, param);
            screen.min + screen.size() * egui::vec2(x, y)
        })
        .collect();
    let accent = ui.visuals().selection.stroke.color;
    let painter = ui.painter().clone();
    // In declaration order, which for a curve is the order of its control polygon.
    painter.add(egui::Shape::dashed_line(
        &at,
        egui::Stroke::new(1.0, accent),
        4.0,
        3.0,
    ));
    let mut moved = None;
    for (param, &pos) in params.iter().zip(&at) {
        let square = egui::Rect::from_center_size(pos, egui::Vec2::splat(RADIUS * 3.0));
        let id = ui.id().with(("ui_builder_point", origin, &param.name));
        let response = ui
            .interact(square, id, egui::Sense::drag())
            .on_hover_text(&param.name)
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
            moved = Some((param, across.x.clamp(0.0, 1.0), across.y.clamp(0.0, 1.0)));
        }
    }
    let Some((param, x, y)) = moved else {
        return;
    };
    let Some(written) = builder.inspect(origin).map(|inspected| inspected.style) else {
        return;
    };
    let mut style = InlineStyle::parse(&written);
    let property = format!("{PARAM_PREFIX}{}", param.name.to_ascii_lowercase());
    style.set(&property, Some(&format!("{x:.3} {y:.3}")));
    builder.set_attribute(origin, "style", &style.write());
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
