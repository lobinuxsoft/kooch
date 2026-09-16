use egui::{Pos2, Rect, Vec2};

use super::*;
use crate::shader_graph::{Graph, Node};

fn map() -> Rect {
    Rect::from_min_size(Pos2::new(500.0, 400.0), SIZE)
}

/// An empty graph has nothing to show, and `Rect::NOTHING` would scale to infinity.
#[test]
fn an_empty_graph_has_no_bounds() {
    assert!(bounds(&Graph::new()).is_none());
}

#[test]
fn the_bounds_hold_every_node() {
    let mut graph = Graph::new();
    graph.insert_node(Pos2::new(-200.0, 0.0), Node::Uv);
    graph.insert_node(Pos2::new(600.0, 300.0), Node::Output);

    let bounds = bounds(&graph).expect("bounds");

    assert!(bounds.contains(Pos2::new(-200.0, 0.0)));
    assert!(
        bounds.contains(Pos2::new(600.0, 300.0) + NODE_SIZE),
        "a node's body fell outside the bounds",
    );
}

/// 🔴 The graph is fitted, never stretched: one scale for both axes, or the minimap is a picture of
/// the bounding box rather than of the graph.
#[test]
fn a_wide_graph_keeps_its_shape() {
    let wide = Rect::from_min_size(Pos2::ZERO, Vec2::new(2000.0, 100.0));
    let (scale, _) = fit(wide, map());

    assert!(
        (scale - map().width() / wide.width()).abs() < 0.0001,
        "the wider axis has to be the one that fits",
    );
}

/// What the click path rests on: a point converted onto the minimap and back is where it started.
#[test]
fn a_point_survives_the_round_trip() {
    let bounds = Rect::from_min_size(Pos2::new(-100.0, -50.0), Vec2::new(900.0, 600.0));
    let (scale, offset) = fit(bounds, map());
    let start = Pos2::new(120.0, 240.0);

    let onto = (start.to_vec2() * scale + offset).to_pos2();
    let back = ((onto.to_vec2() - offset) / scale).to_pos2();

    assert!(
        (back - start).length() < 0.001,
        "a click would land somewhere else: {back:?} from {start:?}",
    );
}

/// The middle of the graph is drawn in the middle of the minimap.
#[test]
fn the_centre_lands_in_the_centre() {
    let bounds = Rect::from_min_size(Pos2::new(40.0, 40.0), Vec2::new(400.0, 300.0));
    let (scale, offset) = fit(bounds, map());

    let centre = (bounds.center().to_vec2() * scale + offset).to_pos2();

    assert!((centre - map().center()).length() < 0.001, "{centre:?}");
}
