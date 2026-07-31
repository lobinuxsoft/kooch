//! [`MeshletRenderStage::resize`] — recreate per-pixel attachments
//! (vbuf / depth / color) and the lazy Hi-Z pyramids when the
//! destination surface changes size. Cull / rasterizer / deferred /
//! material pool are unaffected.

use crate::meshlet::deferred::DEFERRED_COLOR_FORMAT;
use crate::meshlet::vis_buffer::VISIBILITY_BUFFER_FORMAT;

use super::super::{MeshletRenderStage, create_2d_attachment};

impl MeshletRenderStage {
    /// Recreates the stage's vbuf / depth / color textures at
    /// `new_size` if it differs from the current size. The cull,
    /// rasterizer, deferred and material pool are unaffected — only
    /// the per-pixel attachments need to grow with the destination
    /// surface.
    pub fn resize(&mut self, device: &wgpu::Device, new_size: (u32, u32)) {
        if new_size == self.size {
            return;
        }
        assert!(
            new_size.0 > 0 && new_size.1 > 0,
            "MeshletRenderStage::resize requires non-zero dimensions"
        );

        let (vbuf_texture, vbuf_view) = create_2d_attachment(
            device,
            "meshlet_render_stage_vbuf",
            new_size,
            VISIBILITY_BUFFER_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        let (depth_texture, depth_view) = create_2d_attachment(
            device,
            "meshlet_render_stage_depth",
            new_size,
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let depth_sample_view = super::super::depth_sample_view(&depth_texture);
        let (color_texture, color_view) = create_2d_attachment(
            device,
            "meshlet_render_stage_color",
            new_size,
            DEFERRED_COLOR_FORMAT,
            // RENDER_ATTACHMENT: the two-pass material path writes color
            // as a fragment render target; STORAGE_BINDING stays for the
            // compute debug-mode path.
            wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
        );

        // #454: rebuild the triangle-density accumulator in lock-step
        // with the production attachments. Only allocated when the
        // construction-time caps survey installed it — that decision
        // is preserved across resize.
        let (triangle_density_texture, triangle_density_view) =
            if self.triangle_density_texture.is_some() {
                let (tex, view) = create_2d_attachment(
                    device,
                    "meshlet_render_stage_triangle_density",
                    new_size,
                    wgpu::TextureFormat::R32Uint,
                    wgpu::TextureUsages::STORAGE_BINDING
                        | wgpu::TextureUsages::COPY_DST
                        | wgpu::TextureUsages::COPY_SRC,
                );
                (Some(tex), Some(view))
            } else {
                (None, None)
            };

        // Hi-Z pyramids stay lazy (#486) — only recreate them on
        // resize when they were already allocated by some prior
        // SPD-orchestrator hook.
        let old_pyramid_bytes = self.hiz_prev.as_ref().map(|p| p.byte_size()).unwrap_or(0)
            + self.hiz_curr.as_ref().map(|p| p.byte_size()).unwrap_or(0);
        let hiz_prev = if self.hiz_prev.is_some() {
            Some(crate::hi_z::HiZ::new(device, new_size.0, new_size.1))
        } else {
            None
        };
        let hiz_curr = if self.hiz_curr.is_some() {
            Some(crate::hi_z::HiZ::new(device, new_size.0, new_size.1))
        } else {
            None
        };
        let new_pyramid_bytes = hiz_prev.as_ref().map(|p| p.byte_size()).unwrap_or(0)
            + hiz_curr.as_ref().map(|p| p.byte_size()).unwrap_or(0);

        self.vbuf_texture = vbuf_texture;
        self.vbuf_view = vbuf_view;
        self.depth_texture = depth_texture;
        self.depth_view = depth_view;
        self.depth_sample_view = depth_sample_view;
        self.color_texture = color_texture;
        self.color_view = color_view;
        self.triangle_density_texture = triangle_density_texture;
        self.triangle_density_view = triangle_density_view;
        // Retire the OLD pyramids (if any) into the current slot of
        // the triple-buffer rather than dropping them inline. The
        // next frame's render() rotates the index and clears the
        // slot that is now 2 frames old, by which point the GPU is
        // guaranteed to be done. Currently a no-op since the lazy
        // pyramids are still `None`; activates when SPD (#486)
        // turns them on.
        let retire_idx = self.frame_bind_groups_index;
        if let Some(prev_pyramid) = std::mem::replace(&mut self.hiz_prev, hiz_prev) {
            self.retired_pyramids[retire_idx].push(prev_pyramid);
        }
        if let Some(curr_pyramid) = std::mem::replace(&mut self.hiz_curr, hiz_curr) {
            self.retired_pyramids[retire_idx].push(curr_pyramid);
        }
        // Both pyramids are fresh — they need clear_to_far before the
        // next render_with_assets samples them in pass A.
        self.hi_z_initialized = false;
        // #493: keep the atomic R64 vbuf in lockstep with the legacy
        // attachments so both paths stay valid post-resize.
        if let Some(stage) = self.vbuf64_stage.as_mut() {
            stage.resize(device, new_size);
        }
        self.size = new_size;

        if let Some(tracker) = &self.vram_tracker {
            tracker.add(new_pyramid_bytes.saturating_sub(old_pyramid_bytes));
        }
    }
}
