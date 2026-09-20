//! The noises' controls: every input and output wired compiles, and what is left unwired costs
//! nothing (#1159).

use egui::Pos2;
use egui_snarl::{InPinId, NodeId, OutPinId};
use kooch_render::material::Shader;

use crate::shader_graph::*;

fn wire(graph: &mut Graph, from: NodeId, output: usize, to: NodeId, input: usize) {
    graph.connect(OutPinId { node: from, output }, InPinId { node: to, input });
}

/// Feeds every input of `node` a constant and `outputs` of it into the Surface Output. A simplex's
/// tiling is left out: it is refused, and that refusal has a test of its own.
fn wired(node: Node, outputs: &[usize]) -> String {
    let mut graph = Graph::new();
    let inputs = node.inputs().len();
    let skewed = matches!(&node, Node::FractalNoise { basis, .. } if basis == "simplex");
    let added = graph.insert_node(Pos2::ZERO, node);
    let output = graph.insert_node(Pos2::ZERO, Node::surface_output());
    for input in 0..inputs {
        if skewed && input == NOISE_TILING {
            continue;
        }
        let constant = graph.insert_node(Pos2::ZERO, Node::ConstFloat(0.5 + input as f32));
        wire(&mut graph, constant, 0, added, input);
    }
    for (input, &pin) in outputs.iter().enumerate() {
        wire(&mut graph, added, pin, output, input);
    }
    generate(&graph).unwrap()
}

fn compiles(source: &str, name: &str) {
    let shader = Shader::parse(source).unwrap_or_else(|why| panic!("{name} does not parse: {why}"));
    kooch_render::meshlet::validate_surface(&shader.params_wgsl(), &shader.source)
        .unwrap_or_else(|why| panic!("{name} does not compile: {why}"));
}

/// Phase, distortion and colour each pull in code of their own: all of them at once, per basis
/// and fractal, must still be one shader naga accepts.
#[test]
fn every_noise_control_compiles() {
    for basis in NOISE_BASES {
        for fractal in NOISE_FRACTALS {
            let node = Node::FractalNoise {
                basis: basis.to_owned(),
                fractal: fractal.to_owned(),
            };
            let source = wired(node, &[0, NOISE_COLOUR]);

            let name = format!("{basis} {fractal}");
            assert!(
                source.contains(&format!("graph_{basis}_fractal3(")),
                "{name}: no phase"
            );
            assert!(
                source.contains(&format!("graph_{basis}_warp(")),
                "{name}: no distortion"
            );
            compiles(&source, &name);
        }
    }
}

/// Every metric with all five outputs wired, the border's extra pass included.
#[test]
fn every_voronoi_output_compiles() {
    for metric in VORONOI_METRICS {
        let node = Node::VoronoiNoise {
            metric: metric.to_owned(),
        };
        let source = wired(node, &[0, 1, VORONOI_BORDER, 3, 4]);

        assert!(
            source.contains(", true, "),
            "{metric}: the border pass is off"
        );
        compiles(&source, metric);
    }
}

/// 🔴 A noise samples three times for colour and in 3D for phase: left unwired, neither may be
/// paid for, nor may the border pass.
#[test]
fn unwired_noise_controls_cost_nothing() {
    let fractal = wired(
        Node::FractalNoise {
            basis: "simplex".to_owned(),
            fractal: "fbm".to_owned(),
        },
        &[0],
    );
    let mut graph = Graph::new();
    let noise = graph.insert_node(
        Pos2::ZERO,
        Node::FractalNoise {
            basis: "gradient".to_owned(),
            fractal: "ridged".to_owned(),
        },
    );
    let output = graph.insert_node(Pos2::ZERO, Node::surface_output());
    wire(&mut graph, noise, 0, output, 0);
    let bare = generate(&graph).unwrap();
    let voronoi = wired(
        Node::VoronoiNoise {
            metric: "euclidean".to_owned(),
        },
        &[0],
    );

    assert_eq!(
        fractal.matches("graph_simplex_fractal3(vec3").count(),
        1,
        "phase wired, sampled once:\n{fractal}"
    );
    assert_eq!(
        bare.matches("graph_gradient_fractal(l").count(),
        1,
        "sampled more than once:\n{bare}"
    );
    assert!(
        !bare.contains("fn graph_hash3("),
        "an unanimated noise carries the 3D hash"
    );
    assert!(
        !bare.contains("_warp(l"),
        "an unwired distortion is sampled"
    );
    assert!(
        voronoi.contains(", false, "),
        "the border pass runs unwired"
    );
    compiles(&bare, "bare gradient");
}

/// 🔴 A skewed lattice has no period, so a tiled simplex would seam while claiming not to. The graph
/// says so instead of emitting a lie.
#[test]
fn a_tiled_simplex_is_refused() {
    let mut graph = Graph::new();
    let noise = graph.insert_node(
        Pos2::ZERO,
        Node::FractalNoise {
            basis: "simplex".to_owned(),
            fractal: "fbm".to_owned(),
        },
    );
    let output = graph.insert_node(Pos2::ZERO, Node::surface_output());
    let tiling = graph.insert_node(Pos2::ZERO, Node::ConstFloat(4.0));
    wire(&mut graph, noise, 0, output, 0);
    wire(&mut graph, tiling, 0, noise, NOISE_TILING);
    let why = generate(&graph).expect_err("a tiled simplex generated");
    assert!(why.contains("simplex"), "{why}");
}

/// The wrap reaches every lattice read a tiled noise takes, and costs nothing when no noise tiles.
#[test]
fn a_tiled_noise_wraps() {
    let mut graph = Graph::new();
    let noise = graph.insert_node(
        Pos2::ZERO,
        Node::VoronoiNoise {
            metric: "euclidean".to_owned(),
        },
    );
    let output = graph.insert_node(Pos2::ZERO, Node::surface_output());
    let tiling = graph.insert_node(Pos2::ZERO, Node::ConstFloat(4.0));
    wire(&mut graph, noise, 0, output, 0);
    let bare = generate(&graph).unwrap();
    wire(&mut graph, tiling, 0, noise, VORONOI_TILING);
    let tiled = generate(&graph).unwrap();

    assert!(
        bare.contains("vec2<f32>(0.0))"),
        "an untiled noise carries a period:\n{bare}"
    );
    assert!(
        tiled.contains("round(max("),
        "a tiled noise carries no period:\n{tiled}"
    );
    compiles(&tiled, "tiled voronoi");
}
