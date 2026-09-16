//! The shader graph's minimap (#1167): every node, the rectangle currently on screen, and a click
//! to go there.
//!
//! `egui-snarl` has none, so this draws its own from what it does expose — node positions and the
//! live view transform. Godot's `GraphEditMinimap` is the shape: fit the graph's bounds into the
//! corner **keeping the aspect ratio**, a box per node, the camera rectangle over them.

use egui::emath::TSTransform;
use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};

use crate::shader_graph::{Graph, NODE_SIZE};

/// How big the minimap is drawn, and how far it sits from the panel's corner.
const SIZE: Vec2 = Vec2::new(180.0, 120.0);
const MARGIN: f32 = 8.0;

/// Room left around the nodes so one on the edge is not drawn against the frame.
const PADDING: f32 = 40.0;

/// Draws the minimap over the bottom-right of `panel`. Returns the point in graph space the user
/// asked to look at, if they clicked.
///
/// 🔴 In an `Area` of its own, above the graph. `egui-snarl` draws into a sublayer that covers the
/// whole panel and senses drags everywhere on it, so a minimap painted into the panel's own layer
/// would be drawn under the nodes and would never see a click.
pub(crate) fn draw(
    ui: &mut egui::Ui,
    panel: Rect,
    graph: &Graph,
    to_global: TSTransform,
) -> Option<Pos2> {
    let bounds = bounds(graph)?;
    let corner = Pos2::new(panel.max.x - SIZE.x - MARGIN, panel.max.y - SIZE.y - MARGIN);

    let mut looked_at = None;
    egui::Area::new(ui.id().with("graph_minimap"))
        .order(egui::Order::Foreground)
        .fixed_pos(corner)
        .constrain_to(panel)
        .show(ui.ctx(), |ui| {
            let (map, response) = ui.allocate_exact_size(SIZE, Sense::click());
            let (scale, offset) = fit(bounds, map);
            let onto = |at: Pos2| (at.to_vec2() * scale + offset).to_pos2();

            let painter = ui.painter();
            painter.rect_filled(map, 4.0, Color32::from_black_alpha(200));

            for (_, pos, _) in graph.nodes_pos_ids() {
                let node = Rect::from_min_size(onto(pos), NODE_SIZE * scale);
                painter.rect_filled(node, 1.0, Color32::from_gray(150));
            }

            // Where the panel is looking, in graph space.
            let seen = to_global.inverse() * panel;
            painter.rect_stroke(
                Rect::from_min_max(onto(seen.min), onto(seen.max)).intersect(map),
                2.0,
                Stroke::new(1.0, Color32::from_rgb(120, 190, 255)),
                egui::StrokeKind::Inside,
            );

            if response.clicked()
                && let Some(clicked) = response.interact_pointer_pos()
            {
                // Back the other way: where that pixel of the minimap sits in the graph.
                looked_at = Some(((clicked.to_vec2() - offset) / scale).to_pos2());
            }
        });
    looked_at
}

/// The scale and offset that fit `bounds` inside `map` without stretching either axis.
fn fit(bounds: Rect, map: Rect) -> (f32, Vec2) {
    let scale = (map.width() / bounds.width())
        .min(map.height() / bounds.height())
        .min(1.0);
    (
        scale,
        map.center().to_vec2() - bounds.center().to_vec2() * scale,
    )
}

/// What the graph occupies — nodes and their assumed size — with room around it.
fn bounds(graph: &Graph) -> Option<Rect> {
    let mut bounds = Rect::NOTHING;
    for (_, pos, _) in graph.nodes_pos_ids() {
        bounds.extend_with(pos);
        bounds.extend_with(pos + NODE_SIZE);
    }
    bounds.is_finite().then(|| bounds.expand(PADDING))
}

#[cfg(test)]
mod tests;
