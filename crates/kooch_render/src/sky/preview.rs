//! A sky shader drawn for the Shader Graph panel (#1900): the camera turns slowly, so the whole
//! backdrop reads before it is assigned to a `SkyRenderer`.

use std::hash::{Hash, Hasher};

use glam::Vec3;

use super::graph::{SkyGraph, SkyMaterials, SkyView};
use crate::post_process::PreviewMaterials;

/// Radians per second the preview camera turns.
const TURN_RATE: f32 = 0.15;

pub struct SkyPreview {
    graph: SkyGraph,
    /// The pipeline writes depth; sized to the panel's target.
    depth: Option<((u32, u32), wgpu::TextureView)>,
}

impl SkyPreview {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        Self {
            graph: SkyGraph::new(device, format),
            depth: None,
        }
    }

    /// Draws the sky into `target`. `Err` carries why the shader does not compile.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        size: (u32, u32),
        params_wgsl: &str,
        source: &str,
        materials: PreviewMaterials<'_>,
        time: f32,
    ) -> Result<(), String> {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        params_wgsl.hash(&mut hasher);
        source.hash(&mut hasher);
        self.graph
            .ensure(device, hasher.finish(), params_wgsl, source)?;

        let depth = self.depth_view(device, size);
        let yaw = time * TURN_RATE;
        let forward = Vec3::new(yaw.sin(), 0.25, -yaw.cos());
        let aspect = size.0.max(1) as f32 / size.1.max(1) as f32;
        let view = SkyView {
            view: glam::camera::rh::view::look_to_mat4(Vec3::ZERO, forward, Vec3::Y),
            projection: glam::camera::rh::proj::directx::perspective(1.2, aspect, 0.1, 100.0),
            position: Vec3::ZERO,
            time,
        };
        let mut pass = super::SkyRenderPass::begin(encoder, target, &depth);
        self.graph.record(
            device,
            queue,
            &mut pass,
            SkyMaterials {
                pool: materials.pool,
                textures: materials.textures,
                refs: materials.slots,
                slot: 0,
            },
            &view,
        );
        Ok(())
    }

    fn depth_view(&mut self, device: &wgpu::Device, size: (u32, u32)) -> wgpu::TextureView {
        if let Some((built, view)) = &self.depth
            && *built == size
        {
            return view.clone();
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sky_preview_depth"),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: crate::VIEWPORT_DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        self.depth = Some((size, view.clone()));
        view
    }
}
