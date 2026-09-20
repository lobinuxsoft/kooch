//! What a tiled noise answers at the two sides of the uv square (#1237). The graph's WGSL is baked
//! over its own uv on the GPU and read back, because a seam is a number: the step across the edge
//! against the steps inside.
//!
//! Run with:
//!   cargo test -p kooch_editor_core --lib shader_graph::tests::seam

use crate::shader_graph::{Graph, Node, VORONOI_TILING, generate};
use egui::Pos2;
use egui_snarl::{InPinId, NodeId, OutPinId};
use kooch_core::Guid;
use kooch_render::material::{Material, MaterialPipeline, Shader};
use kooch_render::meshlet::trim;

const SIDE: u32 = 256;

fn wire(graph: &mut Graph, from: NodeId, output: usize, to: NodeId, input: usize) {
    graph.connect(OutPinId { node: from, output }, InPinId { node: to, input });
}

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .ok()?;
    // The surface's own group 4 is the textures, and the default limit stops at four groups.
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("seamless_noise"),
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .ok()
}

/// A transparent surface whose alpha is a Voronoi over the uv, tiled `cells` times or not at all.
fn noise_shader(cells: Option<f32>) -> String {
    let mut graph = Graph::new();
    let uv = graph.insert_node(Pos2::ZERO, Node::Uv);
    let noise = graph.insert_node(
        Pos2::ZERO,
        Node::VoronoiNoise {
            metric: "euclidean".to_owned(),
        },
    );
    let output = graph.insert_node(
        Pos2::ZERO,
        Node::ShaderOutput {
            kind: "transparent".to_owned(),
        },
    );
    let scale = graph.insert_node(Pos2::ZERO, Node::ConstFloat(4.0));
    wire(&mut graph, uv, 0, noise, 0);
    wire(&mut graph, scale, 0, noise, 1);
    // The transparent output's alpha, which the bake hands back as it is.
    wire(&mut graph, noise, 0, output, 5);
    if let Some(cells) = cells {
        let tiling = graph.insert_node(Pos2::ZERO, Node::ConstFloat(cells));
        wire(&mut graph, tiling, 0, noise, VORONOI_TILING);
    }
    generate(&graph).expect("the graph generates")
}

/// The same noise read around a turn: the angle is the y of `PolarCoordinates`, so the period goes
/// on that axis and the radius is left open.
fn polar_shader(cells: Option<f32>) -> String {
    let mut graph = Graph::new();
    let uv = graph.insert_node(Pos2::ZERO, Node::Uv);
    let polar = graph.insert_node(Pos2::ZERO, Node::PolarCoordinates);
    let noise = graph.insert_node(
        Pos2::ZERO,
        Node::VoronoiNoise {
            metric: "euclidean".to_owned(),
        },
    );
    let output = graph.insert_node(
        Pos2::ZERO,
        Node::ShaderOutput {
            kind: "transparent".to_owned(),
        },
    );
    let scale = graph.insert_node(Pos2::ZERO, Node::ConstFloat(4.0));
    wire(&mut graph, uv, 0, polar, 0);
    wire(&mut graph, polar, 0, noise, 0);
    wire(&mut graph, scale, 0, noise, 1);
    wire(&mut graph, noise, 0, output, 5);
    if let Some(cells) = cells {
        let tiling = graph.insert_node(
            Pos2::ZERO,
            Node::ConstVector {
                width: 2,
                value: [0.0, cells, 0.0, 0.0],
            },
        );
        wire(&mut graph, tiling, 0, noise, VORONOI_TILING);
    }
    generate(&graph).expect("the graph generates")
}

/// The baked alpha of `source` over its uv square.
fn baked(device: &wgpu::Device, queue: &wgpu::Queue, source: &str) -> Vec<u8> {
    let mut materials = MaterialPipeline::new(device, queue);
    let shader = Guid::new_v4();
    materials.add_shader(shader, &Shader::parse(source).expect("the shader parses"));
    let mut look = Material::new([1.0, 1.0, 1.0, 1.0], 0.0, 1.0, 0.0);
    look.shader = Some(shader);
    let slot = materials.register(queue, Guid::new_v4(), &look);
    trim::bake::mask(device, queue, &materials, slot, SIDE).expect("the surface bakes")
}

/// The mean step between two columns of the bake.
fn step(mask: &[u8], left: u32, right: u32) -> f32 {
    let at = |x: u32, y: u32| mask[(y * SIDE + x) as usize] as f32;
    let sum: f32 = (0..SIDE).map(|y| (at(left, y) - at(right, y)).abs()).sum();
    sum / SIDE as f32
}

/// The mean step between neighbouring columns inside the square: what "no seam" looks like.
fn inside(mask: &[u8]) -> f32 {
    let sum: f32 = (1..SIDE - 1).map(|x| step(mask, x, x + 1)).sum();
    sum / (SIDE - 2) as f32
}

/// 🔴 The reason the issue exists: a noise that does not tile jumps across the edge of the square,
/// and one that tiles crosses it like any other pair of columns.
#[test]
fn a_tiled_noise_has_no_seam() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let open = baked(&device, &queue, &noise_shader(None));
    let tiled = baked(&device, &queue, &noise_shader(Some(4.0)));

    let seam = |mask: &[u8]| step(mask, SIDE - 1, 0);
    let (open_seam, open_inside) = (seam(&open), inside(&open));
    let (tiled_seam, tiled_inside) = (seam(&tiled), inside(&tiled));
    assert!(
        open_seam > open_inside * 4.0,
        "the untiled noise has no seam to fix: {open_seam} across the edge, {open_inside} inside",
    );
    assert!(
        tiled_seam < tiled_inside * 3.0,
        "the tiled noise still seams: {tiled_seam} across the edge, {tiled_inside} inside",
    );
}

/// 🔴 The case the issue was found in: a noise read through polar coordinates cuts along the turn,
/// where the angle comes back to itself. The step across that line is measured against the steps
/// beside it, in the same rows.
#[test]
fn a_tiled_polar_noise_has_no_seam() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    // The angle jumps down the middle, below the centre: atan2 crosses its own edge there.
    let middle = SIDE / 2;
    let across = |mask: &[u8], x: u32| -> f32 {
        let at = |x: u32, y: u32| mask[(y * SIDE + x) as usize] as f32;
        let rows = 4..middle - 4;
        let count = rows.len() as f32;
        rows.map(|y| (at(x - 1, y) - at(x + 1, y)).abs())
            .sum::<f32>()
            / count
    };
    let beside = |mask: &[u8]| -> f32 {
        let spread: f32 = (middle / 4..middle - 8)
            .map(|x| across(mask, x))
            .sum::<f32>();
        spread / (middle - 8 - middle / 4) as f32
    };
    let open = baked(&device, &queue, &polar_shader(None));
    let tiled = baked(&device, &queue, &polar_shader(Some(4.0)));

    assert!(
        across(&open, middle) > beside(&open) * 4.0,
        "the untiled turn has no seam to fix: {} across, {} beside",
        across(&open, middle),
        beside(&open),
    );
    assert!(
        across(&tiled, middle) < beside(&tiled) * 4.0,
        "the tiled turn still seams: {} across, {} beside",
        across(&tiled, middle),
        beside(&tiled),
    );
}
