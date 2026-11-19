//! Engine render plugins.

pub mod assets;

mod core;
mod frame;
mod meshlets;
mod present;

pub use assets::AssetPlugin;
pub use core::{RenderCorePlugin, init_renderers};

use kooch_core::app::App;
use kooch_core::gpu::{GpuContext, TargetDesc, TargetId, TargetPool};
use kooch_core::plugin::Plugin;
use kooch_core::resource::Resources;
use kooch_core::schedule::Order;
use kooch_core::stage::Stage;

use crate::VIEWPORT_DEPTH_FORMAT;

/// The game's render: [`RenderCorePlugin`], and the frame that draws its cameras to the window.
#[derive(Default)]
pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugin(RenderCorePlugin);
        // Before the frame, and in `Render` rather than `Update`: the resource it reads is written
        // by `apply_render_settings_system` in `Update`, and a stage boundary is the only ordering
        // between two plugins that does not depend on which one registered first.
        app.add_system(Stage::Render, apply_presentation_system);
        // 🔴 The frame in three systems, each naming the one before it. The scene is recorded and
        // submitted BEFORE the swapchain image is asked for, and that order is now a constraint
        // rather than three calls inside one function (#392).
        app.add_ordered(
            Stage::Render,
            Order::after("apply_presentation_system"),
            frame::prepare_frame_system,
        );
        app.add_ordered(
            Stage::Render,
            Order::after("prepare_frame_system"),
            meshlets::render_meshlets_system,
        );
        app.add_ordered(
            Stage::Render,
            Order::after("render_meshlets_system"),
            present::present_frame_system,
        );
    }

    fn name(&self) -> &str {
        "RenderPlugin"
    }
}

/// Puts [`Presentation`](crate::quality::Presentation) on the surface.
fn apply_presentation_system(resources: &mut Resources) {
    let Some(wanted) = resources.get::<crate::quality::Presentation>().copied() else {
        return;
    };
    let Some(gpu) = resources.get_mut::<GpuContext>() else {
        return;
    };
    gpu.set_vsync(wanted_vsync(
        wanted.vsync,
        kooch_core::gpu::vsync_override(),
    ));
}

/// The precedence rule, split out so it is testable without a GPU.
fn wanted_vsync(asset: bool, over: Option<bool>) -> bool {
    over.unwrap_or(asset)
}

/// The surface-sized depth target, held from the pool (#392).
pub(super) struct GameDepth {
    target: TargetId,
    size: (u32, u32),
}

impl GameDepth {
    pub(super) fn new(pool: &mut TargetPool, size: (u32, u32)) -> Self {
        Self {
            target: pool.acquire("game_depth_texture", depth_desc(size)),
            size,
        }
    }

    /// Swaps the target for one of the new size. The old one goes back to the pool, which is what
    /// keeps a run of resizes from allocating a depth texture per resize.
    pub(super) fn ensure(&mut self, pool: &mut TargetPool, size: (u32, u32)) {
        if size == self.size {
            return;
        }
        pool.release(self.target);
        self.target = pool.acquire("game_depth_texture", depth_desc(size));
        self.size = size;
    }

    pub(super) fn view(&self, pool: &TargetPool) -> Option<wgpu::TextureView> {
        pool.view(self.target).cloned()
    }
}

fn depth_desc(size: (u32, u32)) -> TargetDesc {
    TargetDesc::attachment(size, VIEWPORT_DEPTH_FORMAT)
        .with_usage(wgpu::TextureUsages::RENDER_ATTACHMENT)
}

#[cfg(test)]
mod tests;
