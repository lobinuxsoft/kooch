use egui::Pos2;
use egui_snarl::{InPinId, OutPinId};
use kooch_render::material::{Shader, ShaderKind};

use super::*;

mod noise;
mod uv;

/// A tint parameter times an albedo texture sampled at the mesh's uv, into base colour.
fn tinted_texture() -> Graph {
    let mut graph = Graph::new();
    let uv = graph.insert_node(Pos2::ZERO, Node::Uv);
    let albedo = graph.insert_node(
        Pos2::ZERO,
        Node::Texture {
            name: "albedo".to_owned(),
            fallback: "white".to_owned(),
            preview: None,
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
    let output = graph.insert_node(Pos2::ZERO, Node::surface_output());
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
    let output = graph.insert_node(Pos2::ZERO, Node::surface_output());
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
    graph.insert_node(Pos2::ZERO, Node::surface_output());
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
    let output = graph.insert_node(Pos2::ZERO, Node::surface_output());
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
    for kind in ShaderKind::NAMES {
        for node in palette() {
            if node.output_kind().is_some() {
                continue;
            }
            let name = format!("{} into {kind}", node.title());
            let mut graph = Graph::new();
            let added = graph.insert_node(Pos2::ZERO, node);
            let output = graph.insert_node(
                Pos2::ZERO,
                Node::ShaderOutput {
                    kind: kind.to_owned(),
                },
            );
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
            assert_eq!(
                source.lines().next(),
                Some(format!("// kind: {kind}").as_str())
            );
            // A post-process has a frame of its own, and no preview yet (#1201).
            if kind == "post_process" {
                kooch_render::meshlet::validate_post(&shader.params_wgsl(), &shader.source)
                    .unwrap_or_else(|why| panic!("{name} does not compile: {why}"));
                continue;
            }
            kooch_render::meshlet::validate_surface(&shader.params_wgsl(), &shader.source)
                .unwrap_or_else(|why| panic!("{name} does not compile: {why}"));
            kooch_render::meshlet::validate_preview(&shader.params_wgsl(), &shader.source)
                .unwrap_or_else(|why| panic!("{name} does not preview: {why}"));
        }
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

/// 🔴 Every noise reads `graph_hash`, and a graph using several must write it once: WGSL refuses a
/// function declared twice. `every_node_compiles` builds one node per graph and cannot see this.
#[test]
fn every_noise_in_one_graph_compiles() {
    let mut graph = Graph::new();
    let output = graph.insert_node(Pos2::ZERO, Node::surface_output());
    let noises = [
        Node::Noise,
        Node::GradientNoise,
        Node::SimplexNoise,
        Node::WhiteNoise,
        Node::Voronoi,
    ];
    for (input, noise) in noises.into_iter().enumerate() {
        let added = graph.insert_node(Pos2::ZERO, noise);
        graph.connect(
            OutPinId {
                node: added,
                output: 0,
            },
            InPinId {
                node: output,
                input,
            },
        );
    }

    let source = generate(&graph).unwrap();

    assert_eq!(
        source.matches("fn graph_hash(").count(),
        1,
        "the hash was written twice"
    );
    let shader = Shader::parse(&source).unwrap();
    kooch_render::meshlet::validate_surface(&shader.params_wgsl(), &shader.source)
        .expect("the noises compile together");
}

/// A node that answers with several things gives each its own pin, and each pin reads its own
/// component — not the whole value packed into one wire, which a colour input reads as red, green
/// and blue (#1170). Every output of Voronoi and Split is wired, which the one-pin test cannot do.
#[test]
fn every_output_is_its_own_value() {
    for (node, expected) in [
        (Node::Voronoi, ["f1", "f2", "edge", "cell"]),
        (Node::Split, ["x", "y", "z", "w"]),
    ] {
        let name = node.title();
        let mut graph = Graph::new();
        let source_node = graph.insert_node(Pos2::ZERO, node);
        let output = graph.insert_node(Pos2::ZERO, Node::surface_output());
        // Four outputs into four of the output's five inputs: base colour, normal, metallic, roughness.
        for pin in 0..4 {
            graph.connect(
                OutPinId {
                    node: source_node,
                    output: pin,
                },
                InPinId {
                    node: output,
                    input: pin,
                },
            );
        }

        let source = generate(&graph).unwrap_or_else(|why| panic!("{name}: {why}"));

        for component in expected {
            assert!(
                source.contains(&format!("vec4<f32>(n0.{component})")),
                "{name}'s {component} output is not read on its own:\n{source}",
            );
        }
        let shader = Shader::parse(&source).unwrap();
        kooch_render::meshlet::validate_surface(&shader.params_wgsl(), &shader.source)
            .unwrap_or_else(|why| panic!("{name} does not compile: {why}"));
    }
}

/// Pin 0 is the whole value; a colour's RGB drops alpha; a channel reads one component.
#[test]
fn each_pin_reads_its_part() {
    let texture = Node::Texture {
        name: "albedo".to_owned(),
        fallback: "white".to_owned(),
        preview: None,
    };
    assert_eq!(Node::Add.output_of("n3", 0), "n3");
    assert_eq!(texture.output_of("n3", 0), "n3");
    assert_eq!(texture.output_of("n3", 1), "vec4<f32>(n3.xyz, 0.0)");
    assert_eq!(texture.output_of("n3", 5), "vec4<f32>(n3.w)");
    assert_eq!(Node::Voronoi.output_of("n3", 2), "vec4<f32>(n3.edge)");
}

/// 🔴 Every pin of every node the menu offers generates a shader naga accepts. Each pin is its own
/// expression, and `every_node_compiles` only ever wires the first.
#[test]
fn every_pin_compiles() {
    for node in palette() {
        for pin in 1..node.outputs().len() {
            let name = format!("{} → {}", node.title(), node.outputs()[pin].0);
            let mut graph = Graph::new();
            let added = graph.insert_node(Pos2::ZERO, node.clone());
            let output = graph.insert_node(Pos2::ZERO, Node::surface_output());
            graph.connect(
                OutPinId {
                    node: added,
                    output: pin,
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
}

/// Every pin the menu offers says how wide it is and what it is for, one entry per pin: a pin
/// added without its line would show an empty tooltip, and one line out of step would explain the
/// wrong pin.
#[test]
fn every_pin_is_documented() {
    for node in palette() {
        let name = node.title();
        assert!(!node.about().is_empty(), "{name} says nothing");
        assert_eq!(
            node.input_docs().len(),
            node.inputs().len(),
            "{name}'s inputs and their docs are out of step"
        );
        for (pin, (_, about)) in node.inputs().iter().zip(node.input_docs()) {
            assert!(!about.is_empty(), "{name}'s {pin} says nothing");
        }
    }
}

/// Graphs written before the Output node had a kind open as a surface (#1179).
#[test]
fn a_legacy_output_is_a_surface() {
    assert_eq!(Node::Output.migrated(), Node::surface_output());
}

/// 🔴 The Scene Color node is what a post-process is for, and it must not break a surface graph
/// that happens to hold one: only a post-process frame declares `sample_scene` (#1201).
#[test]
fn scene_color_reads_only_in_post() {
    let graph_with = |kind: &str| {
        let mut graph = Graph::new();
        let scene = graph.insert_node(Pos2::ZERO, Node::SceneColor);
        let output = graph.insert_node(
            Pos2::ZERO,
            Node::ShaderOutput {
                kind: kind.to_owned(),
            },
        );
        graph.connect(
            OutPinId {
                node: scene,
                output: 0,
            },
            InPinId {
                node: output,
                input: 0,
            },
        );
        generate(&graph).unwrap()
    };

    let post = graph_with("post_process");
    assert!(post.contains("sample_scene("), "{post}");
    let surface = graph_with("surface");
    assert!(!surface.contains("sample_scene("), "{surface}");
}

/// A transparent output writes its alpha, and an unwired one is solid.
#[test]
fn transparent_writes_alpha() {
    let mut graph = Graph::new();
    graph.insert_node(
        egui::pos2(0.0, 0.0),
        Node::ShaderOutput {
            kind: "transparent".to_owned(),
        },
    );
    let source = generate(&graph).unwrap();
    assert!(source.contains("out.alpha = vec4<f32>(1.0).x;"), "{source}");
    let shader = Shader::parse(&source).unwrap();
    assert_eq!(shader.kind, kooch_render::material::ShaderKind::Transparent);
}

/// Wiring alpha clip masks the shader and the result compiles; left unwired, it stays opaque.
#[test]
fn a_wired_clip_masks() {
    for kind in ["surface", "unlit"] {
        let mut graph = Graph::new();
        let uv = graph.insert_node(Pos2::ZERO, Node::Uv);
        let output = graph.insert_node(
            Pos2::ZERO,
            Node::ShaderOutput {
                kind: kind.to_owned(),
            },
        );
        assert!(!Shader::parse(&generate(&graph).unwrap()).unwrap().masked());
        let clip = Node::ShaderOutput {
            kind: kind.to_owned(),
        }
        .inputs()
        .len()
            - 1;
        graph.connect(
            OutPinId {
                node: uv,
                output: 0,
            },
            InPinId {
                node: output,
                input: clip,
            },
        );
        let source = generate(&graph).unwrap();
        let shader = Shader::parse(&source).unwrap();
        assert!(shader.masked(), "{source}");
        kooch_render::meshlet::validate_surface(&shader.params_wgsl(), &shader.source)
            .unwrap_or_else(|why| panic!("{kind} does not compile: {why}\n{source}"));
    }
}

mod typed_params;
