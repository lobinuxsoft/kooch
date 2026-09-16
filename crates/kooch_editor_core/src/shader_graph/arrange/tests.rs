use egui::Pos2;
use egui_snarl::{InPinId, NodeId, OutPinId};

use super::*;
use crate::shader_graph::Node;

/// Wires one node's output into another's input.
fn wire(graph: &mut Graph, from: NodeId, to: NodeId, input: usize) {
    graph.connect(
        OutPinId {
            node: from,
            output: 0,
        },
        InPinId { node: to, input },
    );
}

fn at(graph: &Graph, id: NodeId) -> Pos2 {
    graph.get_node_info(id).expect("the node").pos
}

/// uv → texture → output, all dropped on top of each other.
#[test]
fn a_chain_lays_out_left_to_right() {
    let mut graph = Graph::new();
    let uv = graph.insert_node(Pos2::ZERO, Node::Uv);
    let texture = graph.insert_node(
        Pos2::ZERO,
        Node::Texture {
            name: "albedo".to_owned(),
            fallback: "white".to_owned(),
        },
    );
    let output = graph.insert_node(Pos2::ZERO, Node::Output);
    wire(&mut graph, uv, texture, 0);
    wire(&mut graph, texture, output, 0);

    assert!(arrange(&mut graph), "nothing moved");

    assert!(
        at(&graph, uv).x < at(&graph, texture).x,
        "what feeds a node must sit to its left",
    );
    assert!(at(&graph, texture).x < at(&graph, output).x);
}

/// 🔴 A cycle has no longest path to layer by, and the panel can hold one the codegen would refuse.
#[test]
fn a_cycle_is_left_alone() {
    let mut graph = Graph::new();
    let add = graph.insert_node(Pos2::new(7.0, 11.0), Node::Add);
    wire(&mut graph, add, add, 0);

    assert!(!arrange(&mut graph), "a cycle was laid out");
    assert_eq!(at(&graph, add), Pos2::new(7.0, 11.0), "a node moved");
}

/// Two sources feeding one node share a layer, and must not be drawn on top of each other.
#[test]
fn a_shared_layer_does_not_overlap() {
    let mut graph = Graph::new();
    let a = graph.insert_node(Pos2::ZERO, Node::Uv);
    let b = graph.insert_node(Pos2::ZERO, Node::Time);
    let add = graph.insert_node(Pos2::ZERO, Node::Add);
    wire(&mut graph, a, add, 0);
    wire(&mut graph, b, add, 1);

    arrange(&mut graph);

    let (first, second) = (at(&graph, a), at(&graph, b));
    assert_eq!(first.x, second.x, "one layer, one column");
    assert!(
        (first.y - second.y).abs() >= NODE.y,
        "two nodes of a layer overlap: {first:?} and {second:?}",
    );
}

/// The same graph laid out twice lands in the same place: an arrange that shuffled on every click
/// would be unusable.
#[test]
fn arranging_twice_changes_nothing() {
    let mut graph = Graph::new();
    let uv = graph.insert_node(Pos2::ZERO, Node::Uv);
    let output = graph.insert_node(Pos2::ZERO, Node::Output);
    wire(&mut graph, uv, output, 0);

    assert!(arrange(&mut graph));
    assert!(!arrange(&mut graph), "the layout moved on a second pass");
}
