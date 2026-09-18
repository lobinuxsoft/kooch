use std::collections::HashMap;

use egui_snarl::NodeId;

use super::*;
use crate::shader_graph::Node;

#[test]
fn nothing_writes_no_block() {
    let source = "fn post_process() {}".to_owned();
    assert_eq!(
        embed(source.clone(), &Annotations::default()).unwrap(),
        source
    );
    assert_eq!(extract(&source), Annotations::default());
}

/// A note may hold `*/`, and must come back whole instead of closing its comment.
#[test]
fn annotations_round_trip() {
    let mut annotations = Annotations::default();
    annotations.group(&[0, 2]);
    annotations.note(Pos2::new(10.0, 10.0));
    annotations.notes[0].text = "dither */ here".to_owned();
    let source = embed("fn surface() {}".to_owned(), &annotations).unwrap();
    assert!(source.starts_with("fn surface() {}"));
    assert_eq!(extract(&source), annotations);
}

/// 🔴 Moving a group moves its members and nothing else, however close another node sits.
#[test]
fn a_group_moves_its_members_only() {
    let mut graph = Graph::new();
    let member = graph.insert_node(Pos2::new(50.0, 50.0), Node::Floor);
    let neighbour = graph.insert_node(Pos2::new(60.0, 60.0), Node::Fract);
    let mut annotations = Annotations::default();
    annotations.group(&[member.0]);
    move_group(&annotations, &mut graph, 0, Vec2::new(10.0, 20.0));
    assert_eq!(
        graph.get_node_info(member).unwrap().pos,
        Pos2::new(60.0, 70.0)
    );
    assert_eq!(
        graph.get_node_info(neighbour).unwrap().pos,
        Pos2::new(60.0, 60.0)
    );
}

/// The frame follows the members' drawn rects, with room for the title.
#[test]
fn a_group_fits_its_members() {
    let mut graph = Graph::new();
    let a = graph.insert_node(Pos2::new(0.0, 0.0), Node::Floor);
    let b = graph.insert_node(Pos2::new(300.0, 100.0), Node::Fract);
    let mut annotations = Annotations::default();
    annotations.group(&[a.0, b.0]);
    let rects = HashMap::from([
        (a.0, Rect::from_min_size(Pos2::ZERO, Vec2::new(100.0, 60.0))),
        (
            b.0,
            Rect::from_min_size(Pos2::new(300.0, 100.0), Vec2::new(120.0, 200.0)),
        ),
    ]);
    let frame = fit(&annotations.groups[0], &graph, &rects, 28.0).unwrap();
    assert!(frame.contains_rect(rects[&a.0]) && frame.contains_rect(rects[&b.0]));
    assert!(frame.min.y <= -28.0, "no room for the title: {frame:?}");
}

/// A node is in one group: joining another takes it out of the first, and an emptied group goes.
#[test]
fn a_node_is_in_one_group() {
    let mut annotations = Annotations::default();
    annotations.group(&[1]);
    annotations.group(&[2, 3]);
    annotations.join(1, &[1]);
    assert_eq!(annotations.groups.len(), 1, "the first group emptied");
    assert_eq!(annotations.groups[0].members, [2, 3, 1]);
    annotations.leave(&[2]);
    assert_eq!(annotations.groups[0].members, [3, 1]);
}

/// A removed node leaves its group.
#[test]
fn removed_nodes_are_pruned() {
    let mut graph = Graph::new();
    let a = graph.insert_node(Pos2::ZERO, Node::Floor);
    let b = graph.insert_node(Pos2::ZERO, Node::Fract);
    let mut annotations = Annotations::default();
    annotations.group(&[a.0, b.0]);
    graph.remove_node(b);
    annotations.prune(&graph);
    assert_eq!(annotations.groups[0].members, [a.0]);
    graph.remove_node(NodeId(a.0));
    annotations.prune(&graph);
    assert!(annotations.groups.is_empty());
}
