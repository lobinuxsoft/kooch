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
    annotations.group(Rect::from_min_max(Pos2::ZERO, Pos2::new(300.0, 200.0)));
    annotations.note(Pos2::new(10.0, 10.0));
    annotations.notes[0].text = "dither */ here".to_owned();
    let source = embed("fn surface() {}".to_owned(), &annotations).unwrap();
    assert!(source.starts_with("fn surface() {}"));
    assert_eq!(extract(&source), annotations);
}

/// The group takes the nodes inside it, and leaves the rest.
#[test]
fn a_group_carries_its_nodes() {
    let mut graph = Graph::new();
    let inside = graph.insert_node(Pos2::new(50.0, 50.0), Node::Floor);
    let outside = graph.insert_node(Pos2::new(900.0, 50.0), Node::Fract);
    let mut annotations = Annotations::default();
    annotations.group(Rect::from_min_max(Pos2::ZERO, Pos2::new(300.0, 200.0)));
    move_group(&mut annotations, &mut graph, 0, Vec2::new(10.0, 20.0));
    assert_eq!(
        graph.get_node_info(inside).unwrap().pos,
        Pos2::new(60.0, 70.0)
    );
    assert_eq!(
        graph.get_node_info(outside).unwrap().pos,
        Pos2::new(900.0, 50.0)
    );
    assert_eq!(annotations.groups[0].rect.min, Pos2::new(10.0, 20.0));
}
