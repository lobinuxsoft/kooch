use egui::Pos2;
use egui_snarl::{InPinId, OutPinId};

use super::*;
use crate::shader_graph::Node;

fn pair() -> (Graph, NodeId, NodeId) {
    let mut graph = Graph::new();
    let a = graph.insert_node(Pos2::ZERO, Node::ConstFloat(1.0));
    let b = graph.insert_node(Pos2::new(200.0, 0.0), Node::Fract);
    (graph, a, b)
}

#[test]
fn nothing_changed_is_none() {
    let (graph, ..) = pair();
    assert_eq!(change(&graph, &graph.clone()), None);
}

#[test]
fn structure_is_classified() {
    let (before, a, b) = pair();
    let mut after = before.clone();
    after.insert_node(Pos2::ZERO, Node::Uv);
    assert_eq!(change(&before, &after), Some(GraphStep::AddNodes));

    let mut after = before.clone();
    after.remove_node(b);
    assert_eq!(change(&before, &after), Some(GraphStep::RemoveNodes));

    let mut after = before.clone();
    let wire = (
        OutPinId { node: a, output: 0 },
        InPinId { node: b, input: 0 },
    );
    after.connect(wire.0, wire.1);
    assert_eq!(change(&before, &after), Some(GraphStep::Connect));
    assert_eq!(change(&after, &before), Some(GraphStep::Disconnect));
}

/// A drag and a value edit merge within one gesture; an edit on another node does not.
#[test]
fn continuous_edits_merge() {
    let (before, a, b) = pair();
    let mut after = before.clone();
    after.get_node_info_mut(a).unwrap().pos = Pos2::new(5.0, 5.0);
    assert_eq!(change(&before, &after), Some(GraphStep::Move));

    let mut after = before.clone();
    *after.get_node_mut(a).unwrap() = Node::ConstFloat(2.0);
    assert_eq!(change(&before, &after), Some(GraphStep::Edit(a)));

    let path = std::path::Path::new("/p/a.shader");
    assert_eq!(
        GraphStep::Move.merge_key(path),
        GraphStep::Move.merge_key(path)
    );
    assert_ne!(
        GraphStep::Edit(a).merge_key(path),
        GraphStep::Edit(b).merge_key(path)
    );
    assert_eq!(GraphStep::Connect.merge_key(path), None);
}
