//! [`RenderNode`] trait + helpers.

/// Per-frame state every node may read: render targets, viewport
/// metrics, scene-tree access. Lifetime-borrowed from the orchestrator
/// each frame.
pub struct FrameInfo<'a> {
    /// Final color render target the meshlet / mesh / sky passes paint into.
    pub color_view: &'a wgpu::TextureView,
    /// Depth attachment, if the pass needs one (sky pre-pass, meshlet
    /// rasterizer). Compute-only nodes can ignore it.
    pub depth_view: Option<&'a wgpu::TextureView>,
    /// Width / height of [`Self::color_view`]. Color-target dimensions
    /// the orchestrator already knows; the graph plumbs them so passes
    /// don't recompute aspect from a window handle.
    pub size: (u32, u32),
    /// Wall time since startup in seconds. Used by sky / animation
    /// passes; nodes that don't need it can ignore the field.
    pub time_secs: f32,
    /// ECS world the orchestrator hands every node access to. Read-only
    /// — passes mutate via their owned state, not the resources map.
    pub resources: &'a kooch_core::resource::Resources,
}

impl<'a> FrameInfo<'a> {
    /// Aspect ratio derived from `size`. Returns `1.0` if the height is
    /// zero (e.g. minimised window) so projection matrices stay finite.
    pub fn aspect(&self) -> f32 {
        let (w, h) = self.size;
        if h == 0 { 1.0 } else { w as f32 / h as f32 }
    }
}

/// Per-frame context passed to every render node's execute call.
pub struct RenderContext<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    /// Optional — `None` for tests or for graphs that schedule
    /// scene-agnostic compute. Production paths supply `Some(&FrameInfo)`.
    pub frame: Option<&'a FrameInfo<'a>>,
}

impl<'a> RenderContext<'a> {
    /// Convenience constructor for compute-only / test contexts: sets
    /// `frame` to `None`.
    pub fn compute_only(device: &'a wgpu::Device, queue: &'a wgpu::Queue) -> Self {
        Self {
            device,
            queue,
            frame: None,
        }
    }
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
