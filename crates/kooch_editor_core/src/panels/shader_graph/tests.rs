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

/// 🔴 Every output's name is drawn inside its node and ends clear of its own pin — the first one too,
/// on a node with fields. Measured headless:
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
        Node::VoronoiNoise {
            metric: "euclidean".to_owned(),
        },
        // Not "value": the dropdown would read as the output of that name.
        Node::FractalNoise {
            basis: "gradient".to_owned(),
            fractal: "fbm".to_owned(),
        },
        Node::Float {
            name: "speed".to_owned(),
            default: 1.0,
            range: None,
        },
        Node::WorldPosition,
    ];
    let mut graph = Graph::new();
    for (row, node) in nodes.iter().enumerate() {
        graph.insert_node(Pos2::new(0.0, 260.0 * row as f32), node.clone());
    }
    let outputs: Vec<String> = nodes
        .iter()
        .flat_map(|node| {
            let node = node.clone();
            (0..node.outputs().len())
                .map(move |i| super::pin::label(node.outputs()[i].0, node.output_doc(i).0))
        })
        .collect();
    let inputs: Vec<String> = nodes
        .iter()
        .flat_map(|node| {
            let node = node.clone();
            (0..node.inputs().len())
                .map(move |i| super::pin::label(node.inputs()[i], node.input_docs()[i].0))
        })
        .collect();

    let ctx = egui::Context::default();
    let mut frame = None;
    // egui-snarl sizes a node from the frame before, so the answer is read once it has settled.
    for _ in 0..4 {
        frame = Some(ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 2000.0))),
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

    let mut drawn_names = Vec::new();
    for clipped in &shapes {
        let egui::Shape::Text(text) = &clipped.shape else {
            continue;
        };
        let name = text.galley.text();
        let is_output = outputs.iter().any(|o| o == name);
        if !is_output && !inputs.iter().any(|i| i == name) {
            continue;
        }
        let drawn = Rect::from_min_size(text.pos, text.galley.size());
        // Its own pin is level with it: on its right for an output, on its left for an input.
        let pin = pins
            .iter()
            .filter(|pin| (pin.y - drawn.center().y).abs() < 2.0)
            .filter(|pin| (pin.x > drawn.center().x) == is_output)
            .min_by(|a, b| {
                (a.x - drawn.center().x)
                    .abs()
                    .total_cmp(&(b.x - drawn.center().x).abs())
            })
            .unwrap_or_else(|| panic!("`{name}` is not level with a pin of its side: {drawn:?}"));
        if is_output {
            assert!(
                drawn.right() < pin.x - 7.5,
                "`{name}` runs into its pin: {drawn:?}, pin {pin:?}"
            );
        } else {
            assert!(
                drawn.left() > pin.x + 7.5,
                "`{name}` runs into its pin: {drawn:?}, pin {pin:?}"
            );
        }
        drawn_names.push((name.to_owned(), drawn));
    }
    assert_eq!(
        drawn_names.len(),
        outputs.len() + inputs.len(),
        "not every name was drawn: {drawn_names:?}",
    );
    // 🔴 The node has to grow to hold them: an input's name and an output's on one row must not meet.
    for (i, (first, a)) in drawn_names.iter().enumerate() {
        for (second, b) in &drawn_names[i + 1..] {
            assert!(
                !a.intersects(*b),
                "`{first}` and `{second}` are drawn over each other"
            );
        }
    }
}

/// Resting the pointer on a pin explains it: what it reads and what for (#1159). The pin draws its
/// own tooltip, since egui-snarl hands a pin nothing but a painter.
#[test]
fn a_resting_pointer_explains_pins() {
    use crate::shader_graph::Node;
    use egui_snarl::ui::SnarlWidget;

    let mut graph = Graph::new();
    graph.insert_node(Pos2::new(40.0, 40.0), Node::Panner);
    let ctx = egui::Context::default();
    let mut run = |time: f64, events: Vec<egui::Event>| {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 1100.0))),
                time: Some(time),
                events,
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
                    .id(egui::Id::new("tooltips"))
                    .show(&mut graph, &mut viewer, ui);
            },
        )
    };
    let texts = |frame: &egui::FullOutput| -> Vec<String> {
        frame
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect()
    };
    let mut frame = run(0.0, vec![]);
    for step in 1..4 {
        frame = run(step as f64 * 0.1, vec![]);
    }
    // The speed pin: the second input, the lower of the two leftmost circles.
    let circles: Vec<Pos2> = frame
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Circle(c) => Some(c.center),
            _ => None,
        })
        .collect();
    let left = circles.iter().map(|c| c.x).fold(f32::INFINITY, f32::min);
    let speed = circles
        .iter()
        .filter(|c| (c.x - left).abs() < 1.0)
        .copied()
        .max_by(|a, b| a.y.total_cmp(&b.y))
        .expect("the panner draws its pins");
    let about = "How far it moves per second: x along U, y along V. Time is already applied — \
                 wire a speed, not speed × Time.";

    run(1.0, vec![egui::Event::PointerMoved(speed)]);
    let moving = run(1.05, vec![]);
    let mut rested = run(2.0, vec![]);
    for step in 1..3 {
        rested = run(2.0 + step as f64 * 0.1, vec![]);
    }

    assert!(
        !texts(&moving).iter().any(|t| t == about),
        "shown before the delay"
    );
    assert!(
        texts(&rested).iter().any(|t| t == about),
        "not explained: {:?}",
        texts(&rested)
    );
    assert!(texts(&rested).iter().any(|t| t == "speed (2)"));
}
