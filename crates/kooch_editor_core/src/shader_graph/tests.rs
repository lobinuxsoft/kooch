use egui::Pos2;
use egui_snarl::{InPinId, OutPinId};
use kooch_render::material::Shader;

use super::*;

/// A tint parameter times an albedo texture sampled at the mesh's uv, into base colour.
fn tinted_texture() -> Graph {
    let mut graph = Graph::new();
    let uv = graph.insert_node(Pos2::ZERO, Node::Uv);
    let albedo = graph.insert_node(
        Pos2::ZERO,
        Node::Texture {
            name: "albedo".to_owned(),
            fallback: "white".to_owned(),
        },
    );
    let tint = graph.insert_node(
        Pos2::ZERO,
        Node::Param {
            name: "tint".to_owned(),
            width: 4,
            color: true,
            default: [1.0, 0.5, 0.25, 1.0],
        },
    );
    let multiply = graph.insert_node(Pos2::ZERO, Node::Multiply);
    let output = graph.insert_node(Pos2::ZERO, Node::Output);
    let wire = |graph: &mut Graph, from, to, input| {
        graph.connect(
            OutPinId {
                node: from,
                output: 0,
            },
            InPinId { node: to, input },
        );
    };
    wire(&mut graph, uv, albedo, 0);
    wire(&mut graph, albedo, multiply, 0);
    wire(&mut graph, tint, multiply, 1);
    wire(&mut graph, multiply, output, 0);
    graph
}

/// 🔴 The claim the whole tool rests on: what a graph generates is a shader the engine accepts.
#[test]
fn a_graph_generates_a_valid_shader() {
    let source = generate(&tinted_texture()).unwrap();
    let shader = Shader::parse(&source).expect("the generated shader parses");
    let names: Vec<&str> = shader.params.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["tint", "albedo"]);
    assert_eq!(shader.params[0].default, [1.0, 0.5, 0.25, 1.0]);
    kooch_render::meshlet::validate_surface(&shader.params_wgsl(), &shader.source)
        .expect("the generated shader compiles");
}

/// The graph rides in the file it generated and comes back out of it.
#[test]
fn the_graph_survives_its_own_file() {
    let graph = tinted_texture();
    let source = generate(&graph).unwrap();
    assert!(is_generated(&source));
    let read = extract(&source).expect("the block holds a graph");
    let before: Vec<&Node> = graph.nodes().collect();
    let after: Vec<&Node> = read.nodes().collect();
    assert_eq!(after, before);
}

/// A hand-written shader is not the graph's to open.
#[test]
fn a_handwritten_shader_carries_no_graph() {
    assert!(!is_generated(
        kooch_render::material::DEFAULT_SURFACE_SHADER
    ));
    assert!(extract(kooch_render::material::DEFAULT_SURFACE_SHADER).is_none());
}

/// An unconnected output is zero rather than a compile error: half a graph still renders.
#[test]
fn an_empty_output_still_compiles() {
    let mut graph = Graph::new();
    graph.insert_node(Pos2::ZERO, Node::Output);
    let source = generate(&graph).unwrap();
    let shader = Shader::parse(&source).unwrap();
    assert!(shader.params.is_empty());
    kooch_render::meshlet::validate_surface(&shader.params_wgsl(), &shader.source).unwrap();
}

#[test]
fn a_graph_without_an_output_is_refused() {
    let mut graph = Graph::new();
    graph.insert_node(Pos2::ZERO, Node::Uv);
    assert!(generate(&graph).is_err());
}

/// 🔴 A cycle would recurse until the stack ran out.
#[test]
fn a_cycle_is_refused() {
    let mut graph = Graph::new();
    let add = graph.insert_node(Pos2::ZERO, Node::Add);
    let output = graph.insert_node(Pos2::ZERO, Node::Output);
    graph.connect(
        OutPinId {
            node: add,
            output: 0,
        },
        InPinId {
            node: add,
            input: 0,
        },
    );
    graph.connect(
        OutPinId {
            node: add,
            output: 0,
        },
        InPinId {
            node: output,
            input: 0,
        },
    );
    assert_eq!(
        generate(&graph).unwrap_err(),
        "the graph feeds a node into itself"
    );
}

/// What New Shader Graph writes: it renders like a material without touching a node.
#[test]
fn the_starter_graph_is_a_material() {
    let source = generate(&starter()).unwrap();
    let shader = Shader::parse(&source).unwrap();
    let names: Vec<&str> = shader.params.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["base_color", "roughness", "albedo"]);
    kooch_render::meshlet::validate_surface(&shader.params_wgsl(), &shader.source).unwrap();
}
