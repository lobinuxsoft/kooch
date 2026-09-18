//! What a plugin needs to draw: a pass, the GPU handles it gets, and the targets it draws into
//! (#392).
//!
//! 🔴 This crate exists so [`kooch_plugin_api`] can keep its zero dependencies. A plugin that only
//! moves entities links that one and never compiles wgpu; a plugin that draws links this one too.
//!
//! ```ignore
//! struct Vignette { pipeline: Option<wgpu::RenderPipeline> }
//!
//! impl RenderPass for Vignette {
//!     fn name(&self) -> &str { "vignette" }
//!     fn init(&mut self, setup: PassSetup<'_>) { self.pipeline = Some(build(setup.device)); }
//!     fn record(&mut self, frame: PassFrame<'_>) { /* open a render pass, draw */ }
//! }
//!
//! // In `KoochPlugin::build`:
//! engine.add_pass(Stage::Render, Order::after("render_meshlets_system"), Vignette::default());
//! ```

use kooch_plugin_api::engine_api::Engine;
use kooch_plugin_api::types::{Order, Stage};

/// What makes two render targets interchangeable, so the pool can hand one back instead of
/// allocating. Mirrors the engine's own descriptor — the engine reads this very type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TargetDesc {
    pub size: (u32, u32),
    pub format: wgpu::TextureFormat,
    pub usage: wgpu::TextureUsages,
    pub mips: u32,
    pub samples: u32,
}

impl TargetDesc {
    /// A single-sampled, single-mip colour or depth attachment — what almost every target is.
    pub fn attachment(size: (u32, u32), format: wgpu::TextureFormat) -> Self {
        Self {
            size: (size.0.max(1), size.1.max(1)),
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            mips: 1,
            samples: 1,
        }
    }

    pub fn with_usage(mut self, usage: wgpu::TextureUsages) -> Self {
        self.usage = usage;
        self
    }

    pub fn with_mips(mut self, mips: u32) -> Self {
        self.mips = mips.max(1);
        self
    }
}

/// A target the pool owns. The handle is all a pass holds: the texture stays the pool's, which is
/// what lets it be reused rather than reallocated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TargetId(pub u32);

/// The pool, as a pass sees it.
pub trait Targets {
    fn acquire(&mut self, label: &str, desc: TargetDesc) -> TargetId;

    fn view(&self, target: TargetId) -> Option<&wgpu::TextureView>;

    fn release(&mut self, target: TargetId);
}

/// What a pass gets once, to build its pipelines with.
pub struct PassSetup<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
}

/// What a pass gets each frame: the frame's encoder, the pool, and the handles to write with.
pub struct PassFrame<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub targets: &'a mut dyn Targets,
    pub encoder: &'a mut wgpu::CommandEncoder,
}

/// A pass a plugin adds to the frame.
pub trait RenderPass: Send + Sync + 'static {
    /// What the profiler and the ordering call it. This is the name another pass names in an
    /// [`Order`].
    fn name(&self) -> &str;

    /// Called once, when the GPU is first available.
    fn init(&mut self, setup: PassSetup<'_>) {
        let _ = setup;
    }

    /// Records this pass. Open the passes it needs on `frame.encoder`; the engine submits.
    fn record(&mut self, frame: PassFrame<'_>);
}

/// `add_pass` on every [`Engine`], so a plugin does not handle the erasure itself.
pub trait RenderEngine {
    /// Registers `pass` at `stage`, where `order` puts it. `false` when the host refused it — the
    /// same refusal `add_system` gives outside `build()`.
    fn add_pass(&mut self, stage: Stage, order: Order, pass: impl RenderPass) -> bool;
}

impl<E: Engine + ?Sized> RenderEngine for E {
    fn add_pass(&mut self, stage: Stage, order: Order, pass: impl RenderPass) -> bool {
        // 🔴 Boxed twice on purpose: the host downcasts to `Box<dyn RenderPass>`, which is a
        // concrete type both halves agree on, and `kooch_plugin_api` never names wgpu.
        let erased: Box<dyn RenderPass> = Box::new(pass);
        self.add_pass_erased(stage, order, Box::new(erased))
    }
}
