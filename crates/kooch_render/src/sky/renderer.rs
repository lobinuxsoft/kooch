//! Sky render pass: the `SkyRenderer`'s material, or the engine's own sky graph (#1900).

use kooch_core::Guid;
use kooch_ecs::query::Query;
use kooch_ecs::sky_renderer::SkyRenderer;

use super::default::DefaultSky;
use super::graph::{SkyGraph, SkyView};
use crate::material::MaterialPipeline;

/// Snapshot of an active `SkyRenderer`, so the pass drops its query borrow before drawing.
#[derive(Debug, Clone, Copy)]
pub struct ActiveSky {
    /// A `kind: sky` material; `None` draws the engine's sky.
    pub material: Option<Guid>,
}

impl From<&SkyRenderer> for ActiveSky {
    fn from(s: &SkyRenderer) -> Self {
        Self {
            material: s.material,
        }
    }
}

/// Where the sky draws: the colour and depth it clears, and their aspect.
pub struct SkyTarget<'a> {
    pub view: &'a wgpu::TextureView,
    pub depth: &'a wgpu::TextureView,
    pub aspect: f32,
}

/// Sky render pass, a fullscreen triangle.
pub struct SkyRenderPass {
    graph: SkyGraph,
    default: Option<DefaultSky>,
}

impl SkyRenderPass {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        Self {
            graph: SkyGraph::new(device, format),
            default: DefaultSky::new(device, format),
        }
    }

    /// Picks the highest-priority active `SkyRenderer` in the scene and
    /// returns a snapshot. Returns `None` when no active sky exists.
    pub fn active_sky(resources: &kooch_core::resource::Resources) -> Option<ActiveSky> {
        let query = Query::<&SkyRenderer>::new(resources);
        let mut best: Option<(i32, ActiveSky)> = None;
        query.for_each(|s| {
            if !s.active {
                return;
            }
            let better = match &best {
                Some((p, _)) => s.priority > *p,
                None => true,
            };
            if better {
                best = Some((s.priority, ActiveSky::from(s)));
            }
        });
        best.map(|(_, snap)| snap)
    }

    /// Records the sky into `encoder`, clearing colour and depth: the first pass of the frame when
    /// an active `SkyRenderer` exists.
    #[profiling::function]
    pub fn render(
        &mut self,
        gpu: &kooch_core::gpu::GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        target: SkyTarget<'_>,
        resources: &kooch_core::resource::Resources,
        camera: &crate::ViewCamera,
        sky: ActiveSky,
    ) -> bool {
        let (device, queue) = (gpu.device(), gpu.queue());
        let view = SkyView {
            view: camera.view(),
            projection: camera.projection(target.aspect),
            position: camera.position(),
            time: kooch_core::time::Time::of(resources).elapsed_secs(),
        };
        let mut pass = Self::begin(encoder, target.view, target.depth);
        // Without the material pipeline there are no textures to bind: the clear stays, black.
        let Some(materials) = resources.get::<MaterialPipeline>() else {
            return true;
        };
        let drawn = sky.material.is_some_and(|material| {
            self.graph
                .draw(device, queue, &mut pass, materials, material, &view)
        });
        // A material that cannot draw falls back to the engine's sky, as an unset one does.
        if !drawn && let Some(default) = self.default.as_mut() {
            default.draw(device, queue, &mut pass, materials.texture_pool(), &view);
        }
        true
    }

    /// What a scene without a sky shows: black, the same in the editor and the game (#1900).
    pub fn clear(
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        depth: &wgpu::TextureView,
    ) {
        drop(Self::begin(encoder, target, depth));
    }

    /// The sky's pass: colour cleared to black, depth to far.
    pub(super) fn begin<'e>(
        encoder: &'e mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        depth: &wgpu::TextureView,
    ) -> wgpu::RenderPass<'e> {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("sky_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        })
    }
}
