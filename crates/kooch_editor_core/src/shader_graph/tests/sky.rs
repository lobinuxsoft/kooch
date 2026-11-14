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

    /// A fresh 0..1 from a cell's hash, uncorrelated with the hash itself.
    fn rehash(&mut self, cells: NodeId, scale: f32) -> NodeId {
        let factor = self.float(scale);
        let spread = self.binary(Node::Multiply, (cells, 3), (factor, 0));
        let fract = self.add(Node::Fract);
        self.wire(spread, 0, fract, 0);
        fract
    }

    /// A layer of stars, each with its own temperature and a brightness drawn from the sky's
    /// power law: the count of stars brighter than L falls as L^-1.5.
    fn layer(
        &mut self,
        uv: NodeId,
        cells: [f32; 2],
        radius: f32,
        share: f32,
        faint: f32,
        air: NodeId,
    ) -> NodeId {
        let (star, voronoi) = self.stars(uv, cells, radius, share);
        // Mostly orange to white, rarely blue-white or deep red.
        let pick = self.rehash(voronoi, 3.7);
        let curve = self.float(2.2);
        let skewed = self.binary(Node::Power, (pick, 0), (curve, 0));
        let span = self.float(13000.0);
        let coolest = self.float(2600.0);
        let kelvin = self.add(Node::MulAdd);
        self.wire(skewed, 0, kelvin, 0);
        self.wire(span, 0, kelvin, 1);
        self.wire(coolest, 0, kelvin, 2);
        let hue = self.add(Node::Blackbody);
        self.wire(kelvin, 0, hue, 0);
        // The eye barely sees colour in starlight.
        let washed = self.float(0.35);
        let colour = self.binary(Node::Desaturate, (hue, 0), (washed, 0));

        let draw = self.rehash(voronoi, 13.7);
        let keep = self.float(0.999);
        let least = self.float(0.001);
        let u = self.add(Node::MulAdd);
        self.wire(draw, 0, u, 0);
        self.wire(keep, 0, u, 1);
        self.wire(least, 0, u, 2);
        let slope = self.float(-0.667);
        let law = self.binary(Node::Power, (u, 0), (slope, 0));
        let floor = self.float(faint);
        let level = self.binary(Node::Multiply, (law, 0), (floor, 0));
        let shimmer = self.shimmer(voronoi, air);
        let flicker = self.binary(Node::Multiply, (level, 0), (shimmer, 0));
        let lit = self.binary(Node::Multiply, (star, 0), (flicker, 0));
        self.binary(Node::Multiply, (lit, 0), (colour, 0))
    }

    /// How much the air shakes the light: `twinkle`, strongest at the horizon where the light
    /// crosses the most air.
    fn air(&mut self, normal: NodeId, twinkle: NodeId) -> NodeId {
        let up = self.add(Node::Swizzle {
            pattern: "y".to_owned(),
        });
        self.wire(normal, 0, up, 0);
        let height = self.add(Node::Abs);
        self.wire(up, 0, height, 0);
        let low = self.add(Node::OneMinus);
        self.wire(height, 0, low, 0);
        let reach = self.float(0.7);
        let base = self.float(0.3);
        let weight = self.add(Node::MulAdd);
        self.wire(low, 0, weight, 0);
        self.wire(reach, 0, weight, 1);
        self.wire(base, 0, weight, 2);
        self.binary(Node::Multiply, (weight, 0), (twinkle, 0))
    }

    /// `1 ± air`, each star on its own phase: two sines at unrelated rates, so it never repeats.
    fn shimmer(&mut self, cells: NodeId, air: NodeId) -> NodeId {
        let time = self.add(Node::Time);
        let mut wave = None;
        for (rate, scale) in [(7.0, 6.283), (11.3, 17.9)] {
            let phase = self.rehash(cells, scale);
            let turn = self.float(6.283);
            let offset = self.binary(Node::Multiply, (phase, 0), (turn, 0));
            let speed = self.float(rate);
            let t = self.binary(Node::Multiply, (time, 0), (speed, 0));
            let angle = self.binary(Node::Add, (t, 0), (offset, 0));
            let sine = self.add(Node::Sine);
            self.wire(angle, 0, sine, 0);
            wave = Some(match wave {
                Some(prev) => self.binary(Node::Multiply, (prev, 0), (sine, 0)),
                None => sine,
            });
        }
        let wave = wave.unwrap_or(time);
        let swing = self.binary(Node::Multiply, (wave, 0), (air, 0));
        let one = self.float(1.0);
        self.binary(Node::Add, (swing, 0), (one, 0))
    }
}

/// Stars in two layers and a nebula along a galactic band: the engine's default sky.
pub(crate) fn space() -> Graph {
    let mut b = Builder {
        graph: Graph::new(),
        x: 0.0,
    };
    let uv = b.add(Node::Uv);
    let normal = b.add(Node::WorldNormal);

    // Two layers of stars, each lit and coloured from its cell: few bright, a dust of faint ones.
    let star_brightness = b.add(Node::Float {
        name: "star_brightness".to_owned(),
        default: 1.0,
        range: Some([0.0, 4.0]),
    });
    let twinkle = b.add(Node::Float {
        name: "twinkle".to_owned(),
        default: 0.0,
        range: Some([0.0, 1.0]),
    });
    let air = b.air(normal, twinkle);
    let near = b.layer(uv, [220.0, 110.0], 0.08, 0.4, 0.05, air);
    let far = b.layer(uv, [640.0, 320.0], 0.12, 0.3, 0.015, air);
    let both = b.binary(Node::Add, (near, 0), (far, 0));
    let stars = b.binary(Node::Multiply, (both, 0), (star_brightness, 0));

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
