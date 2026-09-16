//! The small WGSL functions some nodes lean on, each written into a generated shader only when a
//! node in the graph asks for it (#1159). The functions themselves live beside this file as `.wgsl`.

use crate::shader_graph::{Graph, Node};

/// `graph_rotate`, the noises and the shape helpers, each emitted only when a node in the graph
/// asks for it: a generated file carries nothing it does not use.
pub(super) fn helpers(graph: &Graph) -> String {
    let uses = |wanted: fn(&Node) -> bool| graph.node_ids().any(|(_, node)| wanted(node));
    let mut out = String::new();
    for (wanted, body) in [
        (
            (|n| matches!(n, Node::Rotator)) as fn(&Node) -> bool,
            ROTATE_HELPER,
        ),
        (
            |n| matches!(n, Node::Normalize | Node::Reflect | Node::UnpackNormal),
            NORMAL_HELPER,
        ),
        (|n| matches!(n, Node::Blend { .. }), OVERLAY_HELPER),
        // First, and once however many noises the graph has: they all read it.
        (|n| n.is_noise(), HASH_HELPER),
        (|n| matches!(n, Node::Noise), VALUE_NOISE_HELPER),
        (|n| matches!(n, Node::GradientNoise), GRADIENT_NOISE_HELPER),
        (|n| matches!(n, Node::SimplexNoise), SIMPLEX_NOISE_HELPER),
        (|n| matches!(n, Node::Voronoi), VORONOI_HELPER),
        (|n| matches!(n, Node::Rectangle), BOX_HELPER),
        (|n| matches!(n, Node::Ring), RING_HELPER),
        (|n| matches!(n, Node::Polygon), POLYGON_HELPER),
        (|n| matches!(n, Node::Checker), CHECKER_HELPER),
    ] {
        if uses(wanted) {
            out.push_str(body);
            out.push('\n');
        }
    }
    out
}

/// Turns a coordinate around a centre.
const ROTATE_HELPER: &str = include_str!("helpers/rotate.wgsl");

/// 🔴 `normalize` of a zero vector is NaN, and a graph reaches one the moment an input is left
/// unconnected. Up is the answer that keeps rendering.
const NORMAL_HELPER: &str = include_str!("helpers/normal.wgsl");

/// The one blend mode that is not a one-liner.
const OVERLAY_HELPER: &str = include_str!("helpers/overlay.wgsl");

const HASH_HELPER: &str = include_str!("helpers/hash.wgsl");

const VALUE_NOISE_HELPER: &str = include_str!("helpers/value_noise.wgsl");

const GRADIENT_NOISE_HELPER: &str = include_str!("helpers/gradient_noise.wgsl");

const SIMPLEX_NOISE_HELPER: &str = include_str!("helpers/simplex_noise.wgsl");

const VORONOI_HELPER: &str = include_str!("helpers/voronoi.wgsl");

const BOX_HELPER: &str = include_str!("helpers/box.wgsl");

const RING_HELPER: &str = include_str!("helpers/ring.wgsl");

const POLYGON_HELPER: &str = include_str!("helpers/polygon.wgsl");

const CHECKER_HELPER: &str = include_str!("helpers/checker.wgsl");
