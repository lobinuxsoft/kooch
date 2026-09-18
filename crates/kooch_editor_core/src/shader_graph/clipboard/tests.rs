use super::*;

fn float(name: &str) -> Node {
    Node::Float {
        name: name.to_owned(),
        default: 1.0,
        range: None,
    }
}

/// Three nodes in a chain plus the output: a copy of the chain keeps both of its wires, not the
/// one into the output, and not the output itself.
fn chain() -> (Graph, Vec<NodeId>) {
    let mut graph = Graph::new();
    let a = graph.insert_node(Pos2::new(10.0, 20.0), float("levels"));
    let b = graph.insert_node(Pos2::new(200.0, 20.0), Node::Floor);
    let c = graph.insert_node(Pos2::new(400.0, 40.0), Node::Fract);
    let out = graph.insert_node(Pos2::new(600.0, 0.0), Node::surface_output());
    let wire = |graph: &mut Graph, from: NodeId, to: NodeId| {
        graph.connect(
            OutPinId {
                node: from,
                output: 0,
            },
            InPinId { node: to, input: 0 },
        );
    };
    wire(&mut graph, a, b);
    wire(&mut graph, b, c);
    wire(&mut graph, c, out);
    (graph, vec![a, b, c, out])
}

#[test]
fn a_copy_keeps_inner_wires() {
    let (graph, ids) = chain();
    let clip = copy(&graph, &ids).unwrap();
    assert_eq!(clip.nodes.len(), 3, "the output node stays");
    assert_eq!(clip.wires.len(), 2);
    assert_eq!(clip.nodes[0].0, Vec2::ZERO, "relative to the top-left");
}

/// Pasted where asked, wired among themselves, and the parameter renamed.
#[test]
fn a_paste_lands_wired() {
    let (mut graph, ids) = chain();
    let clip = copy(&graph, &ids[..3]).unwrap();
    let placed = paste(&mut graph, &clip, Pos2::new(1000.0, 500.0));
    assert_eq!(placed.len(), 3);
    assert_eq!(
        graph.get_node_info(placed[0]).unwrap().pos,
        Pos2::new(1000.0, 500.0)
    );
    let wires = graph
        .wires()
        .filter(|(out, _)| placed.contains(&out.node))
        .count();
    assert_eq!(wires, 2);
    assert!(matches!(
        graph.get_node(placed[0]),
        Some(Node::Float { name, .. }) if name == "levels_2"
    ));
}

#[test]
fn removing_takes_the_wires() {
    let (mut graph, ids) = chain();
    remove(&mut graph, &ids[1..2]);
    assert_eq!(graph.node_ids().count(), 3);
    assert_eq!(graph.wires().count(), 1);
}

#[test]
fn nothing_copyable_is_none() {
    let (graph, ids) = chain();
    assert_eq!(copy(&graph, &ids[3..]), None);
    assert_eq!(copy(&graph, &[]), None);
}
