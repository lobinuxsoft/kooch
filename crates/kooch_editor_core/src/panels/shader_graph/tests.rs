use egui::{Pos2, Rect, Vec2};

use super::*;

fn panel() -> Rect {
    Rect::from_min_size(Pos2::new(300.0, 120.0), Vec2::new(900.0, 600.0))
}

/// 🔴 Opening a graph shows all of it, centred: a graph that opens off to one side, or only partly on
/// screen, is a graph the user has to go looking for.
#[test]
fn a_graph_opens_framed() {
    let bounds = Rect::from_min_size(Pos2::new(-400.0, -50.0), Vec2::new(2400.0, 900.0));

    let view = framed(bounds, panel());

    let shown = Rect::from_min_max(view * bounds.min, view * bounds.max);
    assert!(
        (shown.center() - panel().center()).length() < 0.01,
        "not centred: {shown:?}"
    );
    assert!(
        panel().expand(0.01).contains_rect(shown),
        "part of the graph is off screen: {shown:?}",
    );
}

/// A small graph is not blown up past actual size to fill the panel.
#[test]
fn a_small_graph_stays_actual_size() {
    let bounds = Rect::from_min_size(Pos2::ZERO, Vec2::new(200.0, 100.0));
    assert_eq!(framed(bounds, panel()).scaling, 1.0);
}

/// 🔴 Every output's name is drawn inside its node and ends clear of its own pin. Measured headless:
/// egui-snarl gives a row past the first a sliver at the node's edge, and a name laid out in it landed
/// outside the node's clip — "sine" at x 711 against a clip ending at 707 — each row 15 points further.
#[test]
fn pin_names_sit_beside_their_pins() {
    use crate::shader_graph::Node;
    use egui_snarl::ui::SnarlWidget;

    let nodes = [
        Node::Texture {
            name: "albedo".to_owned(),
            fallback: "white".to_owned(),
            preview: None,
        },
        Node::Time,
        Node::Voronoi,
        Node::WorldPosition,
    ];
    let mut graph = Graph::new();
    for (row, node) in nodes.iter().enumerate() {
        graph.insert_node(Pos2::new(0.0, 260.0 * row as f32), node.clone());
    }
    let names: Vec<&str> = nodes
        .iter()
        .flat_map(|node| node.outputs().iter().map(|&(name, _)| name))
        .filter(|name| *name != "RGBA")
        .collect();

    let ctx = egui::Context::default();
    let mut frame = None;
    // egui-snarl sizes a node from the frame before, so the answer is read once it has settled.
    for _ in 0..4 {
        frame = Some(ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 1100.0))),
                ..Default::default()
            },
            |ui| {
                let mut viewer = super::viewer::Viewer {
                    fit: None,
                    catalog: &[],
                    drift: Vec2::ZERO,
                    look_at: None,
                    panel: ui.max_rect(),
                    transform: TSTransform::IDENTITY,
                };
                SnarlWidget::new()
                    .id(egui::Id::new("pin_names"))
                    .show(&mut graph, &mut viewer, ui);
            },
        ));
    }
    let shapes = frame.unwrap().shapes;
    let pins: Vec<Pos2> = shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Circle(c) => Some(c.center),
            _ => None,
        })
        .collect();

    let mut seen = 0;
    for clipped in &shapes {
        let egui::Shape::Text(text) = &clipped.shape else {
            continue;
        };
        let name = text.galley.text();
        if !names.contains(&name) {
            continue;
        }
        seen += 1;
        let drawn = Rect::from_min_size(text.pos, text.galley.size());
        assert!(
            clipped.clip_rect.expand(0.5).contains_rect(drawn),
            "`{name}` is drawn outside its node: {drawn:?} against {:?}",
            clipped.clip_rect,
        );
        // Its own pin is the output on its right; the input level with it on the left is not.
        let pin = pins
            .iter()
            .filter(|pin| pin.x > drawn.center().x)
            .min_by(|a, b| {
                (a.y - drawn.center().y)
                    .abs()
                    .total_cmp(&(b.y - drawn.center().y).abs())
            })
            .expect("a pin");
        assert!(
            (pin.y - drawn.center().y).abs() < 2.0,
            "`{name}` is not level with a pin",
        );
        assert!(
            drawn.right() < pin.x - 7.5,
            "`{name}` runs into its pin: ends at {} with the pin at {}",
            drawn.right(),
            pin.x,
        );
    }
    assert!(
        seen >= names.len() - 1,
        "only {seen} of {} names were drawn",
        names.len()
    );
}
