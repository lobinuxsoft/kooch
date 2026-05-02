//! [`RenderNode`] trait + helpers.

/// Per-frame context passed to every render node's execute call.
///
/// PR-1 keeps it minimal. Future growth: graph-managed resources, frame
/// metadata (delta time, frame index), per-pass profiler scope handles.
pub struct RenderContext<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
}

/// Single unit of GPU work scheduled by the [`crate::graph::RenderGraph`].
///
/// Each node owns its pipelines / bind groups / per-pass resources.
/// `execute` records draw / dispatch commands into the shared encoder
/// — barriers between passes are managed by wgpu's encoder.
pub trait RenderNode: Send + 'static {
    fn name(&self) -> &str;
    fn execute(&mut self, ctx: &RenderContext<'_>, encoder: &mut wgpu::CommandEncoder);
}

/// Closure adapter so trivial passes (clear, copy, debug) can be added
/// without a dedicated struct.
pub struct FnNode<F>
where
    F: FnMut(&RenderContext<'_>, &mut wgpu::CommandEncoder) + Send + 'static,
{
    name: String,
    execute: F,
}

impl<F> FnNode<F>
where
    F: FnMut(&RenderContext<'_>, &mut wgpu::CommandEncoder) + Send + 'static,
{
    pub fn new(name: impl Into<String>, execute: F) -> Self {
        Self {
            name: name.into(),
            execute,
        }
    }
}

impl<F> RenderNode for FnNode<F>
where
    F: FnMut(&RenderContext<'_>, &mut wgpu::CommandEncoder) + Send + 'static,
{
    fn name(&self) -> &str {
        &self.name
    }

    fn execute(&mut self, ctx: &RenderContext<'_>, encoder: &mut wgpu::CommandEncoder) {
        (self.execute)(ctx, encoder);
    }
}
