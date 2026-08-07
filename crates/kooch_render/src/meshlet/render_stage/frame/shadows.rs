//! Wiring the shadow pass into a frame (#476).
//!
//! Two steps, in this order and for a reason:
//!
//! 1. [`MeshletRenderStage::prepare_shadows`] runs **before** the
//!    frame's encoder exists, because it can allocate — the atlas on the
//!    first sunlit frame, the cascade culls whenever the scene grows —
//!    and a buffer replaced after a pass references it is a use of the
//!    old one.
//! 2. [`MeshletRenderStage::record_shadows`] runs **first inside** the
//!    encoder, because everything that shades reads the atlas it fills.

use kooch_core::resource::Resources;

use crate::shadow::{PreparedShadows, ShadowPass, ShadowSettings};
use crate::view_camera::ViewCamera;

use super::super::MeshletRenderStage;

impl MeshletRenderStage {
    /// Allocates the atlas if this frame needs one, places the cascades
    /// and sizes the culls.
    ///
    /// `None` when nothing casts: no directional light with
    /// `cast_shadows`, or the author turned shadows off. The caller
    /// passes that straight through to `GpuLights::update`, which leaves
    /// the dummy atlas bound and the sampling switched off.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::meshlet::render_stage) fn prepare_shadows(
        &mut self,
        device: &wgpu::Device,
        resources: &Resources,
        camera: &ViewCamera,
        aspect: f32,
        meshlet_capacity: u32,
        group_capacity: u32,
    ) -> Option<PreparedShadows> {
        let settings = resources
            .get::<ShadowSettings>()
            .copied()
            .unwrap_or_default();
        let sun = kooch_lighting::shadow_casting_sun(resources);

        // Release the atlas when it stops being wanted, or when it was
        // allocated at a resolution the author has since changed. Sixty
        // -four megabytes is worth noticing a settings change over, and
        // a texture cannot be resized in place.
        let texels = settings.clamped_texels();
        if !settings.enabled || sun.is_none() || self.shadow_texels != texels {
            if let Some(released) = self.shadows.take() {
                if let Some(tracker) = self.vram_tracker.as_ref() {
                    tracker.sub(released.atlas_bytes());
                }
                tracing::debug!(
                    target: "kooch_render::shadow",
                    "released the shadow atlas",
                );
            }
            self.shadow_texels = 0;
        }
        let sun = sun.filter(|_| settings.enabled)?;

        let shadows = match self.shadows.as_mut() {
            Some(pass) => pass,
            None => {
                tracing::debug!(
                    target: "kooch_render::shadow",
                    cascade_texels = texels,
                    "allocating the shadow atlas",
                );
                let pass = ShadowPass::new(
                    device,
                    self.cull_pipelines.meshlet_bind_group_layout(),
                    texels,
                    self.config.meshlet_capacity,
                    super::super::super::DEFAULT_MAX_TRIANGLES as u32,
                );
                if let Some(tracker) = self.vram_tracker.as_ref() {
                    tracker.add(pass.atlas_bytes());
                }
                self.shadow_texels = texels;
                self.shadows.insert(pass)
            }
        };

        // Binding is idempotent and lives here rather than at
        // allocation: growing the light buffer rebuilds the bind group,
        // and this is the one call site that runs after every possible
        // rebuild.
        let atlas_view = shadows.atlas_view().clone();
        let prepared = shadows.prepare(
            device,
            camera,
            aspect,
            sun,
            settings.max_distance,
            settings.sun_softness,
            meshlet_capacity,
            group_capacity,
        );
        self.lights.bind_shadow_atlas(device, &atlas_view);
        Some(prepared)
    }

    /// Records the cascade culls and depth passes into this frame's
    /// encoder, ahead of anything that samples them.
    pub(in crate::meshlet::render_stage) fn record_shadows(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        prepared: &PreparedShadows,
        meshlet_bg: &wgpu::BindGroup,
        instance_count: u32,
        max_meshlets_per_mesh: u32,
        lod_target: f32,
    ) {
        let (Some(shadows), Some(pool)) = (self.shadows.as_ref(), self.gpu_pool.as_ref()) else {
            return;
        };
        shadows.record(
            device,
            queue,
            encoder,
            prepared,
            &self.cull_pipelines,
            pool,
            &self.scene,
            meshlet_bg,
            instance_count,
            max_meshlets_per_mesh,
            lod_target,
        );
    }
}
