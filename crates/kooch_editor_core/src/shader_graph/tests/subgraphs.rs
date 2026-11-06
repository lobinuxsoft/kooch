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

fn wire(
    graph: &mut Graph,
    from: egui_snarl::NodeId,
    output: usize,
    to: egui_snarl::NodeId,
    input: usize,
) {
    graph.connect(OutPinId { node: from, output }, InPinId { node: to, input });
}

fn named(graph: &mut Graph, node: Node) -> egui_snarl::NodeId {
    graph.insert_node(Pos2::ZERO, node.into())
}

fn argument(graph: &mut Graph, name: &str) -> egui_snarl::NodeId {
    let name = name.to_owned();
    named(graph, Node::Argument { name })
}

fn result(graph: &mut Graph, name: &str) -> egui_snarl::NodeId {
    let name = name.to_owned();
    named(graph, Node::Result { name })
}

/// #1807's acceptance: a parallax march out of nodes, with no dedicated node.
#[test]
fn a_march_needs_no_node() {
    let mut step = Graph::new();
    let uv = argument(&mut step, "uv");
    let layer = argument(&mut step, "layer");
    let view = argument(&mut step, "view");
    let index = argument(&mut step, "index");
    let image = named(&mut step, Node::UiImage);
    let height = named(&mut step, Node::Luminance);
    let reached = named(&mut step, Node::Step);
    let next_uv = named(&mut step, Node::Subtract);
    let next_layer = named(&mut step, Node::Subtract);
    let (out_uv, out_layer) = (result(&mut step, "uv"), result(&mut step, "layer"));
    let (stop, round) = (result(&mut step, "stop"), result(&mut step, "round"));
    wire(&mut step, uv, 0, image, 0);
    wire(&mut step, image, 0, height, 0);
    wire(&mut step, layer, 0, reached, 0);
    wire(&mut step, height, 0, reached, 1);
    wire(&mut step, uv, 0, next_uv, 0);
    wire(&mut step, view, 0, next_uv, 1);
    wire(&mut step, layer, 0, next_layer, 0);
    wire(&mut step, next_uv, 0, out_uv, 0);
    wire(&mut step, next_layer, 0, out_layer, 0);
    wire(&mut step, reached, 0, stop, 0);
    wire(&mut step, index, 0, round, 0);
    let file = std::path::Path::new("/p/march.subgraph");
    let function = generate_file(&step, file).unwrap();
    let call = subgraph::call_node(file, &function, "march.subgraph".to_owned()).unwrap();

    let mut shader = Graph::new();
    let march = named(&mut shader, call.looped());
    let kind = "ui".to_owned();
    let output = named(&mut shader, Node::ShaderOutput { kind });
    wire(&mut shader, march, 0, output, 0);
    let source = generate(&shader).unwrap();
    assert!(source.contains("break"));
    let used = [(file.to_path_buf(), function)];
    let flat = kooch_core::shader_uses::flatten(&source, &used);
    let parsed = kooch_render::material::Shader::parse(&flat).unwrap();
    kooch_render::ui::validate_ui(&parsed).unwrap();
}

#[test]
fn a_used_texture_joins_the_material() {
    let mut step = Graph::new();
    let uv = argument(&mut step, "uv");
    let texture = Node::Texture {
        name: "heights".to_owned(),
        fallback: "white".to_owned(),
        preview: None,
    };
    let sample = named(&mut step, texture);
    let out = result(&mut step, "uv");
    wire(&mut step, uv, 0, sample, 0);
    wire(&mut step, sample, 0, out, 0);
    let file = std::path::Path::new("/p/march.subgraph");
    let function = generate_file(&step, file).unwrap();
    let call = subgraph::call_node(file, &function, "march.subgraph".to_owned()).unwrap();

    let mut shader = Graph::new();
    let march = named(&mut shader, call.looped());
    let output = named(&mut shader, Node::surface_output());
    wire(&mut shader, march, 0, output, 0);
    let used = [(file.to_path_buf(), function)];
    let source = generate(&shader).unwrap();
    let flat = kooch_core::shader_uses::flatten(&source, &used);
    super::noise::compiles(&flat, "a march over a texture");
}

/// What the menu offers has to compile: the library is only ever read by the panel.
#[test]
fn every_library_function_compiles() {
    for (name, text) in kooch_core::shader_uses::LIBRARY {
        for (node, _) in crate::shader_graph::functions(name, text) {
            let title = node.title();
            let mut shader = Graph::new();
            let call = shader.insert_node(Pos2::ZERO, node.into());
            let output = shader.insert_node(Pos2::ZERO, Node::surface_output().into());
            wire(&mut shader, call, 0, output, 0);
            let source = generate(&shader).unwrap();
            let path = std::path::Path::new("/p/a.shgraph");
            let mut read = |_: &std::path::Path| Err::<Vec<u8>, String>("no file".to_owned());
            let used = kooch_core::shader_uses::used_files(path, &source, &mut read).unwrap();
            let flat = kooch_core::shader_uses::flatten(&source, &used);
            super::noise::compiles(&flat, &title);
        }
    }
}
