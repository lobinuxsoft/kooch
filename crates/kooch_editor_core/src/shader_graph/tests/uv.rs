//! The UV distortions (#1159).

use egui::Pos2;
use egui_snarl::{InPinId, OutPinId};
use kooch_render::material::Shader;

use crate::shader_graph::*;

/// Dropped in with nothing wired, a distortion warps the mesh's uv around its middle — not the
/// corner at zero every other unwired input reads, where it would do nothing visible.
#[test]
fn unwired_distortions_warp_mesh_uv() {
    for node in [
        Node::PolarCoordinates,
        Node::Twirl,
        Node::RadialShear,
        Node::Spherize,
    ] {
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

        let source = generate(&graph).unwrap();

        assert!(
            source.contains("= vec4<f32>(input.uv, 0.0, 0.0).xy - vec4<f32>(0.5).xy;"),
            "{name} does not warp the mesh uv around its middle:\n{source}"
        );
        let shader = Shader::parse(&source).unwrap();
        kooch_render::meshlet::validate_surface(&shader.params_wgsl(), &shader.source)
            .unwrap_or_else(|why| panic!("{name} does not compile: {why}"));
    }
}
