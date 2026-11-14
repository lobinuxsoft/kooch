//! The `sky` kind (#1900): a graph made of the ordinary nodes compiles inside the sky frame.

use egui::Pos2;
use egui_snarl::{InPinId, NodeId, OutPinId};
use kooch_render::material::{Shader, ShaderKind};

use crate::shader_graph::*;

struct Builder {
    graph: Graph,
    x: f32,
}

impl Builder {
    fn add(&mut self, node: Node) -> NodeId {
        self.x += 1.0;
        let pos = Pos2::new((self.x % 6.0) * 240.0, (self.x / 6.0).floor() * 160.0);
        self.graph.insert_node(pos, node.into())
    }

    fn wire(&mut self, from: NodeId, output: usize, to: NodeId, input: usize) {
        self.graph
            .connect(OutPinId { node: from, output }, InPinId { node: to, input });
    }

    fn float(&mut self, value: f32) -> NodeId {
        self.add(Node::ConstFloat(value))
    }

    fn vector(&mut self, value: [f32; 4], width: u32) -> NodeId {
        self.add(Node::ConstVector { width, value })
    }

    /// `node(a, b)` with both wired.
    fn binary(&mut self, node: Node, a: (NodeId, usize), b: (NodeId, usize)) -> NodeId {
        let id = self.add(node);
        self.wire(a.0, a.1, id, 0);
        self.wire(b.0, b.1, id, 1);
        id
    }

    /// Stars from a Voronoi's nearest point: a soft dot of `radius` cells, in a fraction of cells.
    fn stars(&mut self, uv: NodeId, cells: [f32; 2], radius: f32, share: f32) -> (NodeId, NodeId) {
        let tiling = self.vector([cells[0], cells[1], 0.0, 0.0], 2);
        let voronoi = self.add(Node::VoronoiNoise {
            metric: "euclidean".to_owned(),
        });
        self.wire(uv, 0, voronoi, 0);
        self.wire(tiling, 0, voronoi, VORONOI_TILING);
        let low = self.float(0.0);
        let high = self.float(radius);
        let falloff = self.add(Node::Smoothstep);
        self.wire(low, 0, falloff, 0);
        self.wire(high, 0, falloff, 1);
        self.wire(voronoi, 0, falloff, 2);
        let core = self.add(Node::OneMinus);
        self.wire(falloff, 0, core, 0);
        let edge = self.float(1.0 - share);
        let lit = self.binary(Node::Step, (edge, 0), (voronoi, 3));
        let star = self.binary(Node::Multiply, (core, 0), (lit, 0));
        (star, voronoi)
    }
}

/// Stars in two layers and a nebula along a galactic band: what roll-a-ball's sky is made of.
pub(crate) fn space() -> Graph {
    let mut b = Builder {
        graph: Graph::new(),
        x: 0.0,
    };
    let uv = b.add(Node::Uv);
    let normal = b.add(Node::WorldNormal);

    // Bright stars, few, tinted by their cell's hash; then a fine dim layer.
    let (bright, cells) = b.stars(uv, [220.0, 110.0], 0.08, 0.4);
    let exponent = b.float(10.0);
    let rank = b.binary(Node::Power, (cells, 3), (exponent, 0));
    let star_brightness = b.add(Node::Float {
        name: "star_brightness".to_owned(),
        default: 1.6,
        range: Some([0.0, 4.0]),
    });
    let floor = b.float(0.12);
    let level = b.add(Node::MulAdd);
    b.wire(rank, 0, level, 0);
    b.wire(star_brightness, 0, level, 1);
    b.wire(floor, 0, level, 2);
    let warm = b.add(Node::ConstColor([1.0, 0.8, 0.62, 1.0]));
    let cool = b.add(Node::ConstColor([0.7, 0.82, 1.0, 1.0]));
    let tint = b.add(Node::Mix);
    b.wire(warm, 0, tint, 0);
    b.wire(cool, 0, tint, 1);
    b.wire(cells, 3, tint, 2);
    let lit = b.binary(Node::Multiply, (bright, 0), (level, 0));
    let near = b.binary(Node::Multiply, (lit, 0), (tint, 0));
    let (dim, _) = b.stars(uv, [640.0, 320.0], 0.12, 0.3);
    let faint = b.float(0.3);
    let far = b.binary(Node::Multiply, (dim, 0), (faint, 0));
    let stars = b.binary(Node::Add, (near, 0), (far, 0));

    // The galaxy's plane, tilted off the world axes so it never lines up with the horizon.
    let pole = b.vector([0.32, 0.88, 0.35, 0.0], 3);
    let latitude = b.binary(Node::Dot, (normal, 0), (pole, 0));
    let squared = b.binary(Node::Multiply, (latitude, 0), (latitude, 0));
    let width = b.float(-14.0);
    let spread = b.binary(Node::Multiply, (squared, 0), (width, 0));
    let band = b.add(Node::Exp);
    b.wire(spread, 0, band, 0);

    let shape = b.add(Node::FractalNoise {
        basis: "value".to_owned(),
        fractal: "fbm".to_owned(),
    });
    let shape_tiling = b.vector([8.0, 4.0, 0.0, 0.0], 2);
    let shape_octaves = b.float(5.0);
    b.wire(uv, 0, shape, 0);
    b.wire(shape_octaves, 0, shape, 2);
    b.wire(shape_tiling, 0, shape, NOISE_TILING);
    let detail = b.add(Node::FractalNoise {
        basis: "gradient".to_owned(),
        fractal: "fbm".to_owned(),
    });
    let detail_tiling = b.vector([24.0, 12.0, 0.0, 0.0], 2);
    let detail_octaves = b.float(4.0);
    b.wire(uv, 0, detail, 0);
    b.wire(detail_octaves, 0, detail, 2);
    b.wire(detail_tiling, 0, detail, NOISE_TILING);

    let low = b.float(0.45);
    let high = b.float(0.8);
    let patches = b.add(Node::Smoothstep);
    b.wire(low, 0, patches, 0);
    b.wire(high, 0, patches, 1);
    b.wire(shape, 0, patches, 2);
    let band_share = b.float(0.6);
    let milky = b.binary(Node::Multiply, (band, 0), (band_share, 0));
    let glow = b.binary(Node::Add, (patches, 0), (milky, 0));

    let nebula_a = b.add(Node::Color {
        name: "nebula_a".to_owned(),
        default: [0.42, 0.12, 0.36, 1.0],
    });
    let nebula_b = b.add(Node::Color {
        name: "nebula_b".to_owned(),
        default: [0.10, 0.26, 0.45, 1.0],
    });
    let hue = b.add(Node::Mix);
    b.wire(nebula_a, 0, hue, 0);
    b.wire(nebula_b, 0, hue, 1);
    b.wire(detail, 0, hue, 2);
    let intensity = b.add(Node::Float {
        name: "nebula_intensity".to_owned(),
        default: 0.25,
        range: Some([0.0, 1.0]),
    });
    let wisps = b.binary(Node::Multiply, (detail, 0), (intensity, 0));
    let lit_glow = b.binary(Node::Multiply, (glow, 0), (wisps, 0));
    let nebula = b.binary(Node::Multiply, (hue, 0), (lit_glow, 0));

    let deep = b.add(Node::ConstColor([0.004, 0.005, 0.012, 1.0]));
    let sky = b.binary(Node::Add, (nebula, 0), (stars, 0));
    let colour = b.binary(Node::Add, (sky, 0), (deep, 0));
    let output = b.add(Node::ShaderOutput {
        kind: "sky".to_owned(),
    });
    b.wire(colour, 0, output, 0);
    b.graph
}

#[test]
fn a_sky_graph_compiles() {
    let source = generate(&space()).unwrap();
    let shader = Shader::parse(&source).unwrap();
    assert_eq!(shader.kind, ShaderKind::Sky);
    kooch_render::meshlet::validate_sky(&shader.params_wgsl(), &shader.source)
        .unwrap_or_else(|why| panic!("the sky does not compile: {why}"));
}
