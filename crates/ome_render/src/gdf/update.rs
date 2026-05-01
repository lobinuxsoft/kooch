//! Per-frame GDF populate dispatch + plugin wiring.
//!
//! [`GdfPlugin`] adds a `Stage::Startup` system that constructs
//! [`GdfState`] once [`RayMarchRenderer`]'s `BvhState` is in
//! `Resources`, and a `Stage::Render` system that dispatches the
//! cascade-0 populate compute pass each frame from the active
//! camera's world position.
//!
//! PR-3 stays decoupled from the fragment shader: the cascade is
//! populated but the raymarcher does not yet sample it (PR-4).

use glam::{Mat4, Vec3};
use ome_core::app::App;
use ome_core::gpu::GpuContext;
use ome_core::plugin::Plugin;
use ome_core::resource::Resources;
use ome_core::stage::Stage;
use ome_ecs::hierarchy::GlobalTransform;
use ome_ecs::query::Query;
use ome_ecs::PerspectiveCamera;

use super::GdfState;
use crate::raymarch::RayMarchRenderer;

/// Plugin that installs the GDF cascade-0 populate compute pass.
///
/// Construction is deferred to `Stage::Render` if `RayMarchRenderer`
/// is not yet a resource at startup — same lazy-init pattern as
/// `RayMarchPlugin`'s `init_renderer` so plug-in ordering does not
/// silently skip the GDF.
#[derive(Default)]
pub struct GdfPlugin;

impl Plugin for GdfPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(Stage::Startup, init_gdf);
        app.add_system(Stage::Render, update_gdf_system);
    }

    fn name(&self) -> &str {
        "GdfPlugin"
    }
}

/// Construct [`GdfState`] once the dependencies (`GpuContext` +
/// `RayMarchRenderer`) are both available. Idempotent — re-running
/// after the resource lands does nothing if `GdfState` already exists.
fn init_gdf(resources: &mut Resources) {
    if resources.get::<GdfState>().is_some() {
        return;
    }
    let Some(gpu) = resources.get::<GpuContext>() else {
        tracing::warn!("GdfPlugin: GpuContext missing at Startup, deferring init");
        return;
    };
    let Some(renderer) = resources.get::<RayMarchRenderer>() else {
        tracing::warn!(
            "GdfPlugin: RayMarchRenderer missing at Startup, deferring init to first Render tick"
        );
        return;
    };
    let state = GdfState::new(gpu.device(), renderer.bvh_state().buffers());
    resources.insert(state);
    tracing::info!("GdfPlugin: cascade-0 state initialised");
}

/// Per-frame GDF populate dispatch.
///
/// Resource shuffle mirrors `raymarch_plugin::raymarch_system`:
/// `remove` → mutate → `insert`. Submits its own encoder so the
/// populate pass is a stand-alone GPU job — independent of the
/// raymarch encoder lifecycle in PR-3.
pub fn update_gdf_system(resources: &mut Resources) {
    if resources.get::<GdfState>().is_none() {
        // Fallback init: the Startup pass deferred because the
        // renderer was not resident yet. Try again now.
        init_gdf(resources);
    }
    let Some(mut state) = resources.remove::<GdfState>() else {
        return;
    };
    let Some(gpu) = resources.remove::<GpuContext>() else {
        resources.insert(state);
        return;
    };

    let camera_pos = active_camera_world_position(resources).unwrap_or(Vec3::ZERO);

    let mut encoder = gpu
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ome_render::gdf::populate_encoder"),
        });
    state.dispatch_populate(&mut encoder, gpu.queue(), camera_pos);
    gpu.queue().submit(Some(encoder.finish()));

    resources.insert(gpu);
    resources.insert(state);
}

/// Mirror the camera selection from `raymarch::update::update_camera`:
/// highest-priority active `PerspectiveCamera + GlobalTransform` wins.
/// Returns the world-space translation in the simulation frame
/// (composed with `ActiveOrigin` only when PR-9 lands planet-scale).
fn active_camera_world_position(resources: &Resources) -> Option<Vec3> {
    let query = Query::<(&PerspectiveCamera, &GlobalTransform)>::new(resources);
    let mut best: Option<(i32, Mat4)> = None;
    query.for_each(|(cam, gt)| {
        if !cam.active {
            return;
        }
        let better = match &best {
            Some((p, _)) => cam.priority > *p,
            None => true,
        };
        if better {
            best = Some((cam.priority, gt.matrix));
        }
    });
    drop(query);
    best.map(|(_, m)| {
        let (_, _, translation) = m.to_scale_rotation_translation();
        translation
    })
}
