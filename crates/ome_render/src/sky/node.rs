//! [`crate::graph::RenderNode`] adapter around [`super::SkyRenderPass`].
//!
//! Wraps the rich `render(queue, encoder, color, depth, resources,
//! aspect, sky, time)` signature behind the graph's narrow
//! `execute(ctx, encoder)` contract. Per-frame `ActiveSky` lives on
//! the node — orchestrators set it via [`SkyPassNode::set_sky`]
//! before driving the graph.

use super::{ActiveSky, SkyRenderPass};
use crate::graph::{RenderContext, RenderNode};

/// Default sky parameters used when the orchestrator does not set one
/// explicitly. Black-to-black gradient ≈ a clear-to-black pass, which
/// matches the legacy "no SkyRenderer" path.
fn default_active_sky() -> ActiveSky {
    ActiveSky {
        top_color: [0.0, 0.0, 0.0],
        bottom_color: [0.0, 0.0, 0.0],
        sun_direction: [0.0, -1.0, 0.0],
        sun_color: [1.0, 1.0, 1.0],
        cloud_coverage: 0.0,
        cloud_density: 0.0,
        cloud_height: 1.0,
        cloud_thickness: 0.0,
        wind_direction: [1.0, 0.0, 0.0],
        wind_speed: 0.0,
    }
}

/// `RenderNode` wrapper around [`SkyRenderPass`]. Holds the active sky
/// parameters as per-frame state — call [`Self::set_sky`] before
/// dispatching the graph if the scene's sky changes.
pub struct SkyPassNode {
    name: String,
    inner: SkyRenderPass,
    sky: ActiveSky,
}

impl SkyPassNode {
    pub fn new(name: impl Into<String>, inner: SkyRenderPass) -> Self {
        Self {
            name: name.into(),
            inner,
            sky: default_active_sky(),
        }
    }

    /// Replaces the active sky parameters used by the next [`Self::execute`]
    /// call. Cheap — copies a few f32s.
    pub fn set_sky(&mut self, sky: ActiveSky) {
        self.sky = sky;
    }

    pub fn renderer(&self) -> &SkyRenderPass {
        &self.inner
    }

    pub fn renderer_mut(&mut self) -> &mut SkyRenderPass {
        &mut self.inner
    }
}

impl RenderNode for SkyPassNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn execute(&mut self, ctx: &RenderContext<'_>, encoder: &mut wgpu::CommandEncoder) {
        let Some(frame) = ctx.frame else {
            return;
        };
        let Some(depth_view) = frame.depth_view else {
            return;
        };
        let _ = self.inner.render(
            ctx.queue,
            encoder,
            frame.color_view,
            depth_view,
            frame.resources,
            frame.aspect(),
            self.sky,
            frame.time_secs,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_sky_is_black() {
        let s = default_active_sky();
        assert_eq!(s.top_color, [0.0, 0.0, 0.0]);
        assert_eq!(s.bottom_color, [0.0, 0.0, 0.0]);
        assert_eq!(s.cloud_coverage, 0.0);
    }
}
