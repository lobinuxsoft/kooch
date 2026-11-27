//! A planet's ground colliders (#1949) as the Play host's solver holds them. Triangles stay on the
//! GPU and cross the wire once per build (#2030); blocks still pending or empty are boxes.

use std::collections::HashSet;

use glam::{Quat, Vec3};
use kooch_core::gpu::GpuContext;
use kooch_core::resource::Resources;
use kooch_gizmos::{GizmoBatch, WireMeshRenderer, WireMeshes};
use kooch_remote::serde_json::Value;

const BUILT: [u8; 4] = [90, 230, 130, 220];
const EMPTY: Vec3 = Vec3::new(0.45, 0.45, 0.45);
const PENDING: Vec3 = Vec3::new(1.0, 0.8, 0.15);

#[derive(Default)]
pub(crate) struct GroundDebug {
    pool: WireMeshes,
    /// The newest build already in `pool`; the host sends only the triangles past it.
    since: u64,
    renderer: Option<WireMeshRenderer>,
}

/// Brings the pool up to the host's blocks and boxes the ones without triangles.
pub(super) fn update(resources: &mut Resources, on: bool, batch: &mut GizmoBatch) {
    let mut debug = resources.remove::<GroundDebug>().unwrap_or_default();
    let reply = on.then(|| ask(resources, debug.since)).flatten();
    match reply {
        Some(reply) => apply(&mut debug, &reply, batch),
        None if debug.since != 0 => {
            debug.pool.clear();
            debug.since = 0;
        }
        None => {}
    }
    resources.insert(debug);
}

/// Draws the pool over the viewport.
pub(crate) fn render(
    gpu: &GpuContext,
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    depth: &wgpu::TextureView,
    size: (u32, u32),
    resources: &mut Resources,
) {
    let Some(mut debug) = resources.remove::<GroundDebug>() else {
        return;
    };
    if !debug.pool.is_empty() {
        let renderer = debug
            .renderer
            .get_or_insert_with(|| WireMeshRenderer::new(gpu.device(), gpu.format(), None));
        renderer.render(
            gpu.device(),
            gpu.queue(),
            encoder,
            view,
            depth,
            resources,
            &mut debug.pool,
            size,
        );
    }
    resources.insert(debug);
}

/// Draws the pool over the Game panel, from the game's own camera.
pub(crate) fn render_game(
    gpu: &GpuContext,
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    depth: &wgpu::TextureView,
    view_proj: glam::Mat4,
    resources: &mut Resources,
) {
    let Some(mut debug) = resources.remove::<GroundDebug>() else {
        return;
    };
    if !debug.pool.is_empty() {
        let renderer = debug
            .renderer
            .get_or_insert_with(|| WireMeshRenderer::new(gpu.device(), gpu.format(), None));
        renderer.render_as(
            gpu.device(),
            gpu.queue(),
            encoder,
            view,
            depth,
            &mut debug.pool,
            view_proj,
        );
    }
    resources.insert(debug);
}

fn ask(resources: &Resources, since: u64) -> Option<Value> {
    use kooch_remote::protocol::{Method, ResponseData};
    let state = resources.get::<crate::remote_session::RemoteState>()?;
    let session = state.session.as_ref().filter(|_| state.is_connected())?;
    let payload = kooch_remote::serde_json::json!({ "since": since });
    match session.client().call(Method::Extension {
        name: "planet.ground_debug".to_owned(),
        payload,
    }) {
        Ok(ResponseData::Extension { result, .. }) => Some(result),
        _ => None,
    }
}

fn apply(debug: &mut GroundDebug, reply: &Value, batch: &mut GizmoBatch) {
    for mesh in list(reply, "meshes") {
        let Some(id) = mesh.get("id").and_then(Value::as_u64) else {
            continue;
        };
        let vertices: Vec<Vec3> = floats(mesh.get("vertices"))
            .chunks_exact(3)
            .map(Vec3::from_slice)
            .collect();
        let corners: Vec<u32> = mesh
            .get("indices")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|v| Some(v.as_u64()? as u32)).collect())
            .unwrap_or_default();
        let triangles: Vec<[u32; 3]> = corners
            .chunks_exact(3)
            .map(|c| [c[0], c[1], c[2]])
            .collect();
        debug.pool.insert(id, &vertices, &triangles, BUILT);
        debug.since = debug.since.max(id);
    }
    let mut built = HashSet::new();
    let mut missing: Option<u64> = None;
    for block in list(reply, "blocks") {
        let id = block.get("id").and_then(Value::as_u64).unwrap_or(0);
        let state = block.get("state").and_then(Value::as_str).unwrap_or("");
        let position = Vec3::from_slice(&pad::<3>(floats(block.get("position"))));
        let rotation = Quat::from_slice(&pad::<4>(floats(block.get("rotation"))));
        let rotation = if rotation.length_squared() > 0.0 {
            rotation.normalize()
        } else {
            Quat::IDENTITY
        };
        let size = Vec3::from_slice(&pad::<3>(floats(block.get("size"))));
        match state {
            "built" => {
                built.insert(id);
                if !debug.pool.contains(id) {
                    missing = Some(missing.map_or(id, |m| m.min(id)));
                }
                debug.pool.pose(id, position, rotation);
            }
            "empty" => cube(batch, position, rotation, size, EMPTY),
            _ => cube(batch, position, rotation, size, PENDING),
        }
    }
    let stale: Vec<u64> = debug.pool.ids().filter(|id| !built.contains(id)).collect();
    for id in stale {
        debug.pool.remove(id);
    }
    // A host that restarted counts its builds from one again.
    if let Some(id) = missing {
        debug.since = debug.since.min(id.saturating_sub(1));
    }
}

fn list<'a>(reply: &'a Value, key: &str) -> &'a [Value] {
    reply
        .get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn floats(value: Option<&Value>) -> Vec<f32> {
    value
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| Some(v.as_f64()? as f32)).collect())
        .unwrap_or_default()
}

fn pad<const N: usize>(values: Vec<f32>) -> [f32; N] {
    let mut out = [0.0; N];
    for (slot, value) in out.iter_mut().zip(values) {
        *slot = value;
    }
    out
}

/// The twelve edges of a block from its minimum corner.
fn cube(batch: &mut GizmoBatch, corner: Vec3, rotation: Quat, size: Vec3, color: Vec3) {
    let at = |x: f32, y: f32, z: f32| corner + rotation * (size * Vec3::new(x, y, z));
    for a in 0..2 {
        for b in 0..2 {
            let (a, b) = (a as f32, b as f32);
            batch.line(at(0.0, a, b), at(1.0, a, b), color);
            batch.line(at(a, 0.0, b), at(a, 1.0, b), color);
            batch.line(at(a, b, 0.0), at(a, b, 1.0), color);
        }
    }
}
