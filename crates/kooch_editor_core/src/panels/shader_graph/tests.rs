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
                    new_note: None,
                    ungroup: None,
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
                    new_note: None,
                    ungroup: None,
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

/// Runs the panel's widget and canvas frame by frame, as the panel does, and reads back what they
/// did. The graph starts with a grouped node and a neighbour its frame overlaps.
struct Harness {
    ctx: egui::Context,
    graph: Graph,
    annotations: crate::shader_graph::annotations::Annotations,
    member: egui_snarl::NodeId,
    neighbour: egui_snarl::NodeId,
    to_screen: TSTransform,
    time: f64,
}

impl Harness {
    const ID: &str = "harness";

    fn new() -> Self {
        use crate::shader_graph::Node;
        let mut graph = Graph::new();
        let member = graph.insert_node(Pos2::new(60.0, 80.0), Node::Floor);
        let neighbour = graph.insert_node(Pos2::new(90.0, 150.0), Node::Fract);
        let mut annotations = crate::shader_graph::annotations::Annotations::default();
        annotations.group(&[member.0]);
        let mut harness = Self {
            ctx: egui::Context::default(),
            graph,
            annotations,
            member,
            neighbour,
            to_screen: TSTransform::IDENTITY,
            time: 0.0,
        };
        harness.run(vec![]);
        harness.run(vec![]);
        harness
    }

    fn run(&mut self, events: Vec<egui::Event>) {
        use egui_snarl::ui::SnarlWidget;
        self.time += 0.1;
        let (graph, annotations, to_screen) =
            (&mut self.graph, &mut self.annotations, &mut self.to_screen);
        let _ = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 700.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |ui| {
                let area = ui.available_rect_before_wrap();
                let slot = super::canvas::reserve(ui);
                let mut viewer = super::viewer::Viewer {
                    fit: None,
                    catalog: &[],
                    drift: Vec2::ZERO,
                    look_at: None,
                    panel: area,
                    transform: TSTransform::IDENTITY,
                    new_note: None,
                    ungroup: None,
                };
                let id = egui::Id::new(Self::ID);
                SnarlWidget::new().id(id).show(graph, &mut viewer, ui);
                super::canvas::draw(ui, slot, area, annotations, graph, id, viewer.transform);
                *to_screen = viewer.transform;
            },
        );
    }

    /// Presses at `at` (graph space), drags by `by` (screen pixels) and lets go.
    fn drag(&mut self, at: Pos2, by: Vec2) {
        let from = self.to_screen * at;
        let press = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        self.run(vec![egui::Event::PointerMoved(from), press(from, true)]);
        self.run(vec![egui::Event::PointerMoved(from + by * 0.5)]);
        self.run(vec![egui::Event::PointerMoved(from + by)]);
        self.run(vec![press(from + by, false)]);
        self.run(vec![]);
    }

    fn click(&mut self, at: Pos2) {
        let at = self.to_screen * at;
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        self.run(vec![egui::Event::PointerMoved(at), press(true)]);
        self.run(vec![press(false)]);
        self.run(vec![]);
    }

    fn rect(&self, node: egui_snarl::NodeId) -> Rect {
        egui_snarl::ui::get_node_rects(egui::Id::new(Self::ID), &self.ctx)
            .into_iter()
            .find(|(id, _)| *id == node)
            .map(|(_, rect)| rect)
            .expect("the node was drawn")
    }

    fn frame(&self) -> Rect {
        let rects = [self.member, self.neighbour]
            .into_iter()
            .map(|id| (id.0, self.rect(id)))
            .collect();
        crate::shader_graph::annotations::fit(
            &self.annotations.groups[0],
            &self.graph,
            &rects,
            super::canvas::HEADER,
        )
        .unwrap()
    }

    fn pos(&self, node: egui_snarl::NodeId) -> Pos2 {
        self.graph.get_node_info(node).unwrap().pos
    }
}

/// 🔴 A plain click selects a node alone; the published widget only selected with Shift.
#[test]
fn a_click_selects_a_node() {
    let mut harness = Harness::new();
    let title = harness.rect(harness.neighbour).center_top() + Vec2::new(0.0, 8.0);
    harness.click(title);
    assert_eq!(
        egui_snarl::ui::get_selected_nodes(egui::Id::new(Harness::ID), &harness.ctx),
        [harness.neighbour]
    );
}

/// 🔴 Carrying a group over a node moves its member alone, and does not take the node in — even
/// where the frame overlaps it. The frame's handle wins the pointer over the widget beneath it.
#[test]
fn a_group_carries_only_members() {
    let mut harness = Harness::new();
    let frame = harness.frame();
    assert!(
        frame.intersects(harness.rect(harness.neighbour)),
        "the test needs an overlap"
    );
    let (member, neighbour) = (harness.pos(harness.member), harness.pos(harness.neighbour));
    let to_screen = harness.to_screen;

    harness.drag(
        frame.left_top() + Vec2::new(20.0, 8.0),
        Vec2::new(30.0, 10.0),
    );

    assert_eq!(
        harness.to_screen, to_screen,
        "the widget panned: it took the drag"
    );
    let delta = Vec2::new(30.0, 10.0) / to_screen.scaling;
    assert_eq!(harness.pos(harness.member), member + delta);
    assert_eq!(harness.pos(harness.neighbour), neighbour);
    assert_eq!(harness.annotations.groups[0].members, [harness.member.0]);
}

/// Dropping a dragged node inside a group's frame makes it a member.
#[test]
fn a_dropped_node_joins() {
    let mut harness = Harness::new();
    let target = harness.frame().center();
    let grab = harness.rect(harness.neighbour).center_top() + Vec2::new(0.0, 8.0);
    let centre = harness.rect(harness.neighbour).center();
    // Move the node so its centre lands on the frame's, then let go.
    let by = (target - centre) * harness.to_screen.scaling;
    harness.drag(grab, by);
    assert!(
        harness.annotations.groups[0]
            .members
            .contains(&harness.neighbour.0),
        "{:?}",
        harness.annotations.groups
    );
}

/// Double-clicking a group's title edits it in place; Enter keeps what was typed.
#[test]
fn a_group_renames_in_place() {
    let mut harness = Harness::new();
    let title = harness.to_screen * (harness.frame().left_top() + Vec2::new(40.0, 8.0));
    let button = |pressed| egui::Event::PointerButton {
        pos: title,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    harness.run(vec![egui::Event::PointerMoved(title), button(true)]);
    harness.run(vec![button(false)]);
    harness.run(vec![button(true)]);
    harness.run(vec![button(false)]);
    harness.run(vec![]);
    harness.run(vec![egui::Event::Text(" dither".to_owned())]);
    harness.run(vec![egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    harness.run(vec![]);
    assert_eq!(harness.annotations.groups[0].title, "Group dither");
}

/// 🔴 Clicking into the right-click menu's title field keeps the menu open to type in.
#[test]
fn a_group_menu_stays_open() {
    let mut harness = Harness::new();
    let title = harness.to_screen * (harness.frame().left_top() + Vec2::new(40.0, 8.0));
    let button = |pos, button, pressed| egui::Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let secondary = egui::PointerButton::Secondary;
    harness.run(vec![
        egui::Event::PointerMoved(title),
        button(title, secondary, true),
    ]);
    harness.run(vec![button(title, secondary, false)]);
    harness.run(vec![]);
    // The menu opens at the pointer; its first row is the title field.
    let field = title + Vec2::new(40.0, 16.0);
    let primary = egui::PointerButton::Primary;
    harness.run(vec![
        egui::Event::PointerMoved(field),
        button(field, primary, true),
    ]);
    harness.run(vec![button(field, primary, false)]);
    harness.run(vec![]);
    harness.run(vec![egui::Event::Key {
        key: egui::Key::End,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    harness.run(vec![egui::Event::Text(" dither".to_owned())]);
    harness.run(vec![]);
    assert_eq!(harness.annotations.groups[0].title, "Group dither");
}

/// A note's text and Delete live in its right-click menu, so the note must see secondary clicks.
#[test]
fn a_note_edits_from_its_menu() {
    let mut harness = Harness::new();
    harness.annotations.note(Pos2::new(-60.0, 200.0));
    harness.run(vec![]);
    let note = harness.to_screen * Pos2::new(-50.0, 205.0);
    let button = |pos, button, pressed| egui::Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let secondary = egui::PointerButton::Secondary;
    harness.run(vec![
        egui::Event::PointerMoved(note),
        button(note, secondary, true),
    ]);
    harness.run(vec![button(note, secondary, false)]);
    harness.run(vec![]);
    let field = note + Vec2::new(40.0, 16.0);
    let primary = egui::PointerButton::Primary;
    harness.run(vec![
        egui::Event::PointerMoved(field),
        button(field, primary, true),
    ]);
    harness.run(vec![button(field, primary, false)]);
    harness.run(vec![]);
    harness.run(vec![egui::Event::Text("!".to_owned())]);
    harness.run(vec![]);
    assert!(harness.annotations.notes[0].text.contains('!'));
}
