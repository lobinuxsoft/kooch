//! A plugin's render pass, as a [`GpuSystem`] (#392).
//!
//! 🔴 The bridge is one downcast: `kooch_plugin_api` names no GPU type, so a pass arrives erased as
//! `Box<dyn Any>` holding a `Box<dyn RenderPass>` — a concrete type both halves agree on because
//! they share a compiler and the same `kooch_plugin_render`.

use kooch_plugin_render::{PassFrame, PassSetup, RenderPass};

use crate::resource::Resources;
use crate::system::{Frame, GpuSystem};

/// A plugin pass, run by the schedule like any GPU system.
pub(super) struct PluginPass {
    pass: Box<dyn RenderPass>,
    initialized: bool,
}

impl PluginPass {
    /// Unwraps what [`Engine::add_pass_erased`](kooch_plugin_api::engine_api::Engine::add_pass_erased)
    /// was handed. `None` when it holds something else, which is a mismatched plugin build.
    pub(super) fn from_erased(erased: Box<dyn std::any::Any + Send + Sync>) -> Option<Self> {
        let pass = erased.downcast::<Box<dyn RenderPass>>().ok()?;
        Some(Self {
            pass: *pass,
            initialized: false,
        })
    }
}

impl GpuSystem for PluginPass {
    fn init(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        self.pass.init(PassSetup { device, queue });
        self.initialized = true;
    }

    fn prepare(&mut self, _: &wgpu::Device, _: &wgpu::Queue, _: &Resources) {}

    fn record(&mut self, frame: Frame<'_>, encoder: &mut wgpu::CommandEncoder) {
        self.pass.record(PassFrame {
            device: frame.device,
            queue: frame.queue,
            targets: frame.targets,
            encoder,
        });
    }

    fn name(&self) -> &str {
        self.pass.name()
    }

    fn is_initialized(&self) -> bool {
        self.initialized
    }
}

#[cfg(test)]
mod tests;
