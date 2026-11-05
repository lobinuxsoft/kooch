//! The small WGSL functions some nodes lean on, each written into a generated shader only when a
//! node in the graph asks for it (#1159). The functions themselves live beside this file as `.wgsl`.

use egui_snarl::NodeId;

use crate::shader_graph::{Graph, NOISE_PHASE, Node};

/// `graph_rotate`, the noises and the shape helpers, each emitted only when a node in the graph
/// asks for it: a generated file carries nothing it does not use.
pub(super) fn helpers(graph: &Graph) -> String {
    // Old noises read as the nodes they became, so they pull in the same helpers.
    let nodes: Vec<(NodeId, Node)> = graph
        .node_ids()
        .map(|(id, cell)| (id, cell.node.clone().migrated()))
        .collect();
    let uses = |wanted: &dyn Fn(&Node) -> bool| nodes.iter().any(|(_, node)| wanted(node));
    // A fractal noise samples in 3D only when its phase is wired.
    let animated = |basis: &str| {
        nodes.iter().any(|(id, node)| {
            matches!(node, Node::FractalNoise { basis: b, .. } if b == basis)
                && graph
                    .wires()
                    .any(|(_, to)| to.node == *id && to.input == NOISE_PHASE)
        })
    };
    let noise = |basis: &'static str| move |n: &Node| matches!(n, Node::FractalNoise { basis: b, .. } if b == basis);
    let mut out = String::new();
    let mut push = |wanted: bool, body: &str| {
        if wanted {
            out.push_str(body);
            out.push('\n');
        }
    };
    push(uses(&|n| matches!(n, Node::Rotator)), ROTATE_HELPER);
    push(
        uses(&|n| matches!(n, Node::Normalize | Node::Reflect | Node::UnpackNormal)),
        NORMAL_HELPER,
    );
    push(uses(&|n| matches!(n, Node::Blend { .. })), OVERLAY_HELPER);
    // First, and once however many noises the graph has: they all read them, and the wrap is called
    // by the hashes below it (#1237).
    push(uses(&|n| n.is_noise()), WRAP_HELPER);
    push(uses(&|n| n.is_noise()), HASH_HELPER);
    let bases = [
        ("value", VALUE_NOISE_HELPER, VALUE_NOISE3_HELPER),
        ("gradient", GRADIENT_NOISE_HELPER, GRADIENT_NOISE3_HELPER),
        ("simplex", SIMPLEX_NOISE_HELPER, SIMPLEX_NOISE3_HELPER),
    ];
    push(
        bases.iter().any(|(basis, ..)| animated(basis)),
        HASH3_HELPER,
    );
    for (basis, flat, deep) in bases {
        push(uses(&noise(basis)), flat);
        push(animated(basis), deep);
    }
    push(
        uses(&|n| matches!(n, Node::VoronoiNoise { .. })),
        VORONOI_HELPER,
    );
    push(uses(&|n| matches!(n, Node::Rectangle)), BOX_HELPER);
    push(uses(&|n| matches!(n, Node::Ring)), RING_HELPER);
    push(uses(&|n| matches!(n, Node::Polygon)), POLYGON_HELPER);
    push(uses(&|n| matches!(n, Node::Checker)), CHECKER_HELPER);
    push(uses(&|n| matches!(n, Node::RoundedBox)), ROUNDED_BOX_HELPER);
    push(uses(&|n| matches!(n, Node::Rainbow)), RAINBOW_HELPER);
    push(uses(&|n| matches!(n, Node::UiPoint)), UI_POINT_HELPER);
    push(uses(&|n| matches!(n, Node::BlinnPhong)), BLINN_PHONG_HELPER);
    push(
        uses(&|n| matches!(n, Node::SdfFill | Node::SdfStroke)),
        SDF_FILL_HELPER,
    );
    push(
        uses(&|n| matches!(n, Node::Parallax | Node::ImageNormal)),
        HEIGHT_HELPER,
    );
    push(uses(&|n| matches!(n, Node::Parallax)), PARALLAX_HELPER);
    push(
        uses(&|n| matches!(n, Node::ImageNormal)),
        IMAGE_NORMAL_HELPER,
    );
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

/// The lattice period every noise wraps against (#1237).
const WRAP_HELPER: &str = include_str!("helpers/wrap.wgsl");

/// The 3D hash an animated noise walks through.
const HASH3_HELPER: &str = include_str!("helpers/hash3.wgsl");

const VALUE_NOISE_HELPER: &str = include_str!("helpers/value_noise.wgsl");

const VALUE_NOISE3_HELPER: &str = include_str!("helpers/value_noise3.wgsl");

const GRADIENT_NOISE_HELPER: &str = include_str!("helpers/gradient_noise.wgsl");

const GRADIENT_NOISE3_HELPER: &str = include_str!("helpers/gradient_noise3.wgsl");

const SIMPLEX_NOISE_HELPER: &str = include_str!("helpers/simplex_noise.wgsl");

const SIMPLEX_NOISE3_HELPER: &str = include_str!("helpers/simplex_noise3.wgsl");

const VORONOI_HELPER: &str = include_str!("helpers/voronoi.wgsl");

const BOX_HELPER: &str = include_str!("helpers/box.wgsl");

const RING_HELPER: &str = include_str!("helpers/ring.wgsl");

const POLYGON_HELPER: &str = include_str!("helpers/polygon.wgsl");

const CHECKER_HELPER: &str = include_str!("helpers/checker.wgsl");

const ROUNDED_BOX_HELPER: &str = include_str!("helpers/rounded_box.wgsl");

const RAINBOW_HELPER: &str = include_str!("helpers/rainbow.wgsl");
const UI_POINT_HELPER: &str = include_str!("helpers/ui_point.wgsl");
const BLINN_PHONG_HELPER: &str = include_str!("helpers/blinn_phong.wgsl");
const SDF_FILL_HELPER: &str = include_str!("helpers/sdf_fill.wgsl");

/// The image's brightness: what Parallax and Image Normal read as height.
const HEIGHT_HELPER: &str = include_str!("helpers/height.wgsl");

const PARALLAX_HELPER: &str = include_str!("helpers/parallax.wgsl");

const IMAGE_NORMAL_HELPER: &str = include_str!("helpers/image_normal.wgsl");
