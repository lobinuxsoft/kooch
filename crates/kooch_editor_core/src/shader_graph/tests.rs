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
        Node::Color {
            name: "tint".to_owned(),
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

/// 🔴 From the #1159 smoke test: a colour tick on a three-wide parameter wrote `@color` on a
/// `vec3<f32>`, which the engine refuses. The file was written anyway, failed to reload, and the
/// Inspector went on listing the parameters from before it.
#[test]
fn a_narrow_color_param_parses() {
    let mut graph = Graph::new();
    let emissive = graph.insert_node(
        Pos2::ZERO,
        Node::Param {
            name: "emissive".to_owned(),
            width: 3,
            color: true,
            default: [1.0, 0.5, 0.25, 0.0],
        },
    );
    let output = graph.insert_node(Pos2::ZERO, Node::Output);
    graph.connect(
        OutPinId {
            node: emissive,
            output: 0,
        },
        InPinId {
            node: output,
            input: 4,
        },
    );

    let source = generate(&graph).expect("the graph generates a shader");
    let shader = Shader::parse(&source).expect("the generated shader parses");

    assert_eq!(shader.params[0].name, "emissive");
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

/// 🔴 What the whole library rests on: every node the menu offers generates a shader naga accepts,
/// with its inputs left unconnected — which is the state a node is in the moment it is dropped on
/// the canvas, and the one a user sees first.
#[test]
fn every_node_compiles() {
    for node in palette() {
        if matches!(node, Node::Output) {
            continue;
        }
        let name = node.title();
        let mut graph = Graph::new();
        let added = graph.insert_node(Pos2::ZERO, node);
        let output = graph.insert_node(Pos2::ZERO, Node::Output);
        graph.connect(
            OutPinId {
                node: added,
                output: 0,
            },
            InPinId {
                node: output,
                input: 0,
            },
        );

        let source =
            generate(&graph).unwrap_or_else(|why| panic!("{name} generates nothing: {why}"));
        let shader =
            Shader::parse(&source).unwrap_or_else(|why| panic!("{name} does not parse: {why}"));
        kooch_render::meshlet::validate_surface(&shader.params_wgsl(), &shader.source)
            .unwrap_or_else(|why| panic!("{name} does not compile: {why}"));
    }
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

/// 🔴 Graphs written before #1170 carry `Param { width, color }` in their file. They must open as the
/// typed nodes they mean — or every shader authored until then opens with its parameters gone.
#[test]
fn an_old_param_opens_typed() {
    let migrate = |width, color| {
        Node::Param {
            name: "p".to_owned(),
            width,
            color,
            default: [0.25; 4],
        }
        .migrated()
    };
    assert!(matches!(
        migrate(1, false),
        Node::Float {
            default: 0.25,
            range: None,
            ..
        }
    ));
    assert!(matches!(migrate(3, true), Node::Vector { width: 3, .. }));
    assert!(matches!(migrate(4, true), Node::Color { .. }));
    assert!(matches!(migrate(4, false), Node::Vector { width: 4, .. }));
}

/// And through the real door: an old file's graph comes back out of `extract` already typed.
#[test]
fn an_old_file_extracts_typed() {
    let mut graph = Graph::new();
    graph.insert_node(
        Pos2::ZERO,
        Node::Param {
            name: "tint".to_owned(),
            width: 4,
            color: true,
            default: [1.0; 4],
        },
    );
    graph.insert_node(Pos2::ZERO, Node::Output);
    let source = generate(&graph).unwrap();

    let read = extract(&source).expect("the graph");

    assert!(read.nodes().any(|n| matches!(n, Node::Color { .. })));
    assert!(!read.nodes().any(|n| matches!(n, Node::Param { .. })));
}

/// Each typed node reaches the engine as the kind the Inspector draws: a slider, a stepped slider, a
/// picker.
#[test]
fn typed_params_keep_their_editors() {
    use kooch_render::material::ParamKind;

    let mut graph = Graph::new();
    for node in [
        Node::Float {
            name: "amount".to_owned(),
            default: 0.5,
            range: Some([0.0, 2.0]),
        },
        Node::Int {
            name: "sides".to_owned(),
            default: 6.0,
            range: Some([3.0, 12.0]),
        },
        Node::Color {
            name: "tint".to_owned(),
            default: [1.0; 4],
        },
        Node::Vector {
            name: "offset".to_owned(),
            width: 2,
            default: [0.0; 4],
        },
        Node::Output,
    ] {
        graph.insert_node(Pos2::ZERO, node);
    }

    let shader = Shader::parse(&generate(&graph).unwrap()).unwrap();
    let kind = |name: &str| {
        let param = shader.params.iter().find(|p| p.name == name).unwrap();
        (param.kind, param.range)
    };

    assert_eq!(kind("amount"), (ParamKind::Float, Some([0.0, 2.0])));
    assert_eq!(kind("sides"), (ParamKind::Int, Some([3.0, 12.0])));
    assert_eq!(kind("tint"), (ParamKind::Color, None));
    assert_eq!(kind("offset"), (ParamKind::Vec2, None));
}
