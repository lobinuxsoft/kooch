//! The relief-shaping nodes, read back through the planet's CPU evaluator: what the WGSL computes,
//! not a Rust copy of it (#1943).

use bevy_lookup_curve::{Knot, KnotInterpolation, LookupCurve};
use egui::Pos2;
use egui_snarl::{InPinId, OutPinId};
use kooch_render::material::Shader;
use kooch_terrain::{Generator, Shape};

use crate::shader_graph::*;

const SHAPE: Shape = Shape {
    radius: 10.0,
    relief: 10.0,
    depth: 10.0,
    voxel: 0.5,
    lod_distance: 32.0,
};

/// Altitude into `node`'s first input, `pins` typed on the rest, into a planet's output.
fn planet(node: Node, pins: &[(usize, f32)]) -> (Graph, Generator) {
    let mut graph = Graph::new();
    let altitude = graph.insert_node(Pos2::ZERO, Node::SphereBase.into());
    let added = graph.insert_node(Pos2::ZERO, node.into());
    let output = graph.insert_node(
        Pos2::ZERO,
        Node::ShaderOutput {
            kind: "terrain".to_owned(),
        }
        .into(),
    );
    let wire = |graph: &mut Graph, node, into| {
        graph.connect(
            OutPinId { node, output: 0 },
            InPinId {
                node: into,
                input: 0,
            },
        );
    };
    wire(&mut graph, altitude, added);
    wire(&mut graph, added, output);
    for &(input, value) in pins {
        graph
            .get_node_mut(added)
            .unwrap()
            .set_pin(input, [value; 4]);
    }
    let source = generate(&graph).unwrap();
    let shader = Shader::parse(&source).unwrap();
    kooch_terrain::validate_generator(&shader.source).unwrap();
    (graph, Generator::parse(&shader.source).unwrap())
}

fn at(generator: &Generator, altitude: f64) -> f32 {
    let point = glam::DVec3::new(0.0, 0.0, SHAPE.radius + altitude);
    generator.sample(&SHAPE, point, 0).unwrap()
}

/// Every interpolation, a weighted span among them, ends held flat.
fn every_span() -> GraphCurve {
    let knot = |x: f32, y: f32, interpolation, weight: Option<f32>| {
        let mut knot = Knot {
            interpolation,
            ..Knot::default()
        };
        knot.position.x = x;
        knot.position.y = y;
        knot.right_tangent.slope = 2.0;
        knot.right_tangent.weight = weight;
        knot.left_tangent.slope = -1.0;
        knot
    };
    GraphCurve(LookupCurve::new(vec![
        knot(0.0, 0.0, KnotInterpolation::Constant, None),
        knot(1.0, 1.0, KnotInterpolation::Linear, None),
        knot(2.0, -0.5, KnotInterpolation::Cubic, None),
        knot(3.0, 1.0, KnotInterpolation::Cubic, Some(0.7)),
        knot(4.0, 2.0, KnotInterpolation::Linear, None),
    ]))
}

#[test]
fn a_curve_reads_as_drawn() {
    let curve = every_span();
    let (_, generator) = planet(Node::Curve(curve.clone()), &[]);
    for i in 0..=120 {
        // Off the knots: on one, a constant span and the next disagree by definition.
        let x = -1.013 + 6.0 * i as f32 / 120.0;
        let (gpu, cpu) = (at(&generator, f64::from(x)), curve.0.lookup(x));
        // The weighted span is cut into Hermites, so it is close, not exact.
        assert!(
            (gpu - cpu).abs() < 2e-3,
            "at {x}: generated {gpu}, drawn {cpu}"
        );
    }
}

#[test]
fn a_curve_survives_its_file() {
    let (graph, _) = planet(Node::Curve(every_span()), &[]);
    let read = extract(&generate(&graph).unwrap()).unwrap();
    let before: Vec<_> = graph.nodes().collect();
    let after: Vec<_> = read.nodes().collect();
    assert_eq!(after, before);
}

/// Rounded where the two meet, plain far from it; `k` 0 is the plain one.
#[test]
fn smooth_min_and_max_round() {
    let (_, min) = planet(Node::SmoothMin, &[(2, 1.0)]);
    let (_, max) = planet(Node::SmoothMax, &[(2, 1.0)]);
    let (_, plain) = planet(Node::SmoothMin, &[(2, 0.0)]);
    assert!((at(&min, 0.0) + 0.25).abs() < 1e-5);
    assert!((at(&max, 0.0) - 0.25).abs() < 1e-5);
    assert!((at(&min, 3.0) - 0.0).abs() < 1e-5);
    assert!((at(&min, -3.0) + 3.0).abs() < 1e-5);
    assert!(at(&plain, 0.0).abs() < 1e-5);
}

/// Flat for the first half of each step, a smoothstep over the second.
#[test]
fn terrace_steps_and_ramps() {
    let (_, terrace) = planet(Node::Terrace, &[(1, 2.0), (2, 0.5)]);
    for (altitude, height) in [
        (0.5, 0.0),
        (2.5, 2.0),
        (3.5, 3.0),
        (-0.5, -1.0),
        (-1.5, -2.0),
    ] {
        let got = at(&terrace, altitude);
        assert!(
            (got - height).abs() < 1e-4,
            "at {altitude}: {got}, not {height}"
        );
    }
}

/// `every_node_compiles` stops at an older node that fails (#1901), before reaching these.
#[test]
fn shaping_nodes_compile_in_a_surface() {
    for node in [
        Node::SmoothMin,
        Node::SmoothMax,
        Node::Terrace,
        Node::Curve(every_span()),
    ] {
        let name = node.title();
        let mut graph = Graph::new();
        let added = graph.insert_node(Pos2::ZERO, node.into());
        let output = graph.insert_node(Pos2::ZERO, Node::surface_output().into());
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
        let shader = Shader::parse(&generate(&graph).unwrap()).unwrap();
        kooch_render::meshlet::validate_surface(&shader.params_wgsl(), &shader.source)
            .unwrap_or_else(|why| panic!("{name} does not compile: {why}"));
    }
}
