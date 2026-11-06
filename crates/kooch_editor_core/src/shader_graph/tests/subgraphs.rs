use egui::Pos2;
use egui_snarl::{InPinId, OutPinId};

use crate::shader_graph::{Graph, Node, generate, generate_file, subgraph};

/// A shader and a subgraph can each carry the same noise, and the module has to declare it once.
#[test]
fn a_subgraph_call_validates() {
    let mut inner = subgraph::starter();
    let noise = inner.insert_node(Pos2::ZERO, Node::WhiteNoise.into());
    let result = Node::Result {
        name: "grain".to_owned(),
    };
    let grain = inner.insert_node(Pos2::ZERO, result.into());
    inner.connect(
        OutPinId {
            node: noise,
            output: 0,
        },
        InPinId {
            node: grain,
            input: 0,
        },
    );
    let file = std::path::Path::new("/p/ripple.subgraph");
    let function = generate_file(&inner, file).unwrap();
    let call = subgraph::call_node(file, &function, "ripple.subgraph".to_owned()).unwrap();

    let mut outer = Graph::new();
    let called = outer.insert_node(Pos2::ZERO, call.into());
    let own_noise = outer.insert_node(Pos2::ZERO, Node::WhiteNoise.into());
    let multiply = outer.insert_node(Pos2::ZERO, Node::Multiply.into());
    let output = outer.insert_node(Pos2::ZERO, Node::surface_output().into());
    let pins = [
        (called, 1, multiply, 0),
        (own_noise, 0, multiply, 1),
        (multiply, 0, output, 0),
    ];
    for (from, pin, to, input) in pins {
        outer.connect(
            OutPinId {
                node: from,
                output: pin,
            },
            InPinId { node: to, input },
        );
    }
    let source = generate(&outer).unwrap();
    assert!(source.contains("// use: ripple.subgraph"));
    let used = [(file.to_path_buf(), function)];
    let flat = kooch_core::shader_uses::flatten(&source, &used);
    super::noise::compiles(&flat, "a subgraph call");
}

#[test]
fn a_material_parameter_stays_out() {
    let mut graph = subgraph::starter();
    let float = Node::Float {
        name: "amount".to_owned(),
        default: 0.5,
        range: None,
    };
    graph.insert_node(Pos2::ZERO, float.into());
    let file = std::path::Path::new("/p/ripple.subgraph");
    assert!(generate_file(&graph, file).is_err());
}
