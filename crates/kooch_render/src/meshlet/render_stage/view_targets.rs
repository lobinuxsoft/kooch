//! Everything the meshlet path needs *once per view*.
//!
//! # Why this is its own thing
//!
//! [`MeshletRenderStage`](super::MeshletRenderStage) held these fields
//! directly, which was correct while there was exactly one view. There is
//! about to be more than one — a Game panel beside the editor's View
//! (#592), and after that split-screen, minimaps, security cameras,
//! portals — and the two halves of the stage scale differently:
//!
//! - The geometry pool is **global**. A hundred instances of a mesh are
//!   one entry, and `measure_mesh_pool` puts the engine's whole current
//!   asset set at 6.33 MiB. Duplicating it per view buys nothing.
//! - These attachments are **per view**, unavoidably: two views have two
//!   sizes and two framebuffers.
//!
//! Nanite draws the same line. A frame there is a *list of views* — every
//! Virtual Shadow Map tile is one — rendered against a single geometry
//! pool. What multiplies is the view.
//!
//! # The one that is not a texture
//!
//! `hiz_prev` / `hiz_curr` are occlusion state carried **between frames**,
//! and that makes them per view in a way that is easy to miss: two views
//! sharing one pyramid would test this frame's geometry against the other
//! view's depth. Bevy has that bug today — over-culling with overlapping
//! viewports from different cameras, `bevyengine/bevy#15182`.
//!
//! It cannot bite yet, because the pyramids are `None` until #486 turns
//! the two-pass orchestrator on. Putting them here now is what stops it
//! from biting then.

use crate::hi_z::HiZ;
use crate::meshlet::caps::MeshletDebugCaps;
use crate::meshlet::deferred::DEFERRED_COLOR_FORMAT;
use crate::meshlet::dispatcher::MeshletCull;
use crate::meshlet::vbuf64_stage::Vbuf64Stage;
use crate::meshlet::vis_buffer::VISIBILITY_BUFFER_FORMAT;
use crate::vbuf64::Vbuf64Support;

use super::helpers::{create_2d_attachment, depth_sample_view};

/// The attachments, occlusion state and cull buffers backing one view.
pub(crate) struct MeshletView {
    /// This view's cull output. The pipelines that write it are shared
    /// by every view and live on the stage; what lands in these buffers
    /// depends on this view's camera, so they belong here.
    pub(crate) cull: MeshletCull,

    pub(crate) vbuf_texture: wgpu::Texture,
    pub(crate) vbuf_view: wgpu::TextureView,

    pub(crate) depth_texture: wgpu::Texture,
    pub(crate) depth_view: wgpu::TextureView,
    /// Depth-only view of the same depth texture, for `cs_copy_depth` in
    /// the Hi-Z builder. Sampling requires `TextureAspect::DepthOnly`
    /// while the render attachment uses `All`; one view for both roles
    /// would fail wgpu validation in the worst case.
    pub(crate) depth_sample_view: wgpu::TextureView,

    pub(crate) color_texture: wgpu::Texture,
    pub(crate) color_view: wgpu::TextureView,

    /// Per-pixel R32Uint atomic accumulator (#454) backing the
    /// `TriangleDensity` / `Overdraw` heatmaps and the reject overlay.
    /// `Some` only where the device exposes `Features::TEXTURE_ATOMIC`.
    pub(crate) triangle_density_texture: Option<wgpu::Texture>,
    pub(crate) triangle_density_view: Option<wgpu::TextureView>,

    /// Atomic R64 visibility-buffer path (#493). Per view because it
    /// carries its own depth target at this view's size.
    pub(crate) vbuf64_stage: Option<Vbuf64Stage>,

    /// Twin Hi-Z pyramids for the 2-pass cull (#445). Pass A samples
    /// `hiz_prev` (last frame's depth); pass B rebuilds `hiz_curr` from
    /// pass A's raster and re-tests its rejects. The orchestrator swaps
    /// them at end of frame.
    ///
    /// Lazy: `None` until #486's SPD-backed orchestrator switches them
    /// on. The current single-pass path never samples them, so
    /// allocating at construction would waste VRAM and surface wgpu
    /// noise from the editor's per-frame placeholder stage.
    pub(crate) hiz_prev: Option<HiZ>,
    pub(crate) hiz_curr: Option<HiZ>,
    /// `false` until `clear_to_far` has run on a freshly created
    /// `hiz_prev`. Reset by [`MeshletView::resize`], since both
    /// pyramids are recreated and need re-init before pass A samples a
    /// "nothing occluded" pyramid.
    pub(crate) hi_z_initialized: bool,
    /// Pyramids retired by a resize that may still be in flight.
    /// Triple-buffered to defer the drop until the GPU has stopped
    /// using the views — Mesa radv invalidates bind groups dropped
    /// while in flight.
    pub(crate) retired_pyramids: [Vec<HiZ>; 3],

    /// What the blit presents.
    pub(crate) size: (u32, u32),
    /// What the scene is rasterised at. Equal to `size` unless a
    /// technique upscales — see [`MeshletView::new`].
    pub(crate) render_size: (u32, u32),
}

impl MeshletView {
    /// Allocates one view's attachments.
    ///
    /// 🔴 `size` is what reaches the window; `render_size` is what the
    /// scene is rasterised at (#481 step 4). Everything that costs per
    /// PIXEL — the visibility buffer, depth, the Hi-Z pyramids and every
    /// target inside the R64 stage — is allocated at `render_size`.
    /// Only `color_texture`, which the blit presents, stays at `size`.
    ///
    /// That is the whole performance argument: at 67 % of the width the
    /// shading pass evaluates 44 % of the pixels.
    pub(crate) fn new(
        device: &wgpu::Device,
        size: (u32, u32),
        render_size: (u32, u32),
        debug_caps: MeshletDebugCaps,
        vbuf64: Vbuf64Support,
        meshlet_bgl: &wgpu::BindGroupLayout,
        meshlet_capacity: u32,
        max_triangles_per_meshlet: u32,
    ) -> Self {
        assert!(size.0 > 0 && size.1 > 0, "MeshletView size must be > 0");

        let (vbuf_texture, vbuf_view) = create_2d_attachment(
            device,
            "meshlet_view_vbuf",
            render_size,
            VISIBILITY_BUFFER_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        let (depth_texture, depth_view) = create_2d_attachment(
            device,
            "meshlet_view_depth",
            render_size,
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let depth_sample_view = depth_sample_view(&depth_texture);
        let (color_texture, color_view) = create_2d_attachment(
            device,
            "meshlet_view_color",
            size,
            DEFERRED_COLOR_FORMAT,
            // RENDER_ATTACHMENT: the two-pass material path writes color
            // as a fragment target; STORAGE_BINDING stays for the
            // compute debug-mode path.
            wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
        );

        let (triangle_density_texture, triangle_density_view) =
            Self::create_density(device, render_size, debug_caps.supports_texture_atomic());

        let vbuf64_stage = if vbuf64.is_supported() {
            Some(Vbuf64Stage::new(
                device,
                meshlet_bgl,
                wgpu::TextureFormat::Depth32Float,
                render_size,
                size,
                None,
            ))
        } else {
            None
        };

        Self {
            cull: MeshletCull::new(device, meshlet_capacity, max_triangles_per_meshlet),
            vbuf_texture,
            vbuf_view,
            depth_texture,
            depth_view,
            depth_sample_view,
            color_texture,
            color_view,
            triangle_density_texture,
            triangle_density_view,
            vbuf64_stage,
            hiz_prev: None,
            hiz_curr: None,
            hi_z_initialized: false,
            retired_pyramids: [Vec::new(), Vec::new(), Vec::new()],
            size,
            render_size,
        }
    }

    /// The density accumulator, when the device can run it.
    ///
    /// Split out because construction and resize have to make the same
    /// decision, and a resize that forgot the caps gate would allocate a
    /// texture on an adapter whose driver cannot atomically write it.
    fn create_density(
        device: &wgpu::Device,
        size: (u32, u32),
        supported: bool,
    ) -> (Option<wgpu::Texture>, Option<wgpu::TextureView>) {
        if !supported {
            return (None, None);
        }
        let (tex, view) = create_2d_attachment(
            device,
            "meshlet_view_triangle_density",
            size,
            wgpu::TextureFormat::R32Uint,
            wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
        );
        (Some(tex), Some(view))
    }

    /// Recreates the attachments at `new_size`.
    ///
    /// Returns the change in pyramid bytes, for the caller's VRAM
    /// tracker: this type does not own the tracker, because the tracker
    /// counts the whole engine and a view is one contributor to it.
    ///
    /// `retire_index` is the caller's frame slot — retired pyramids park
    /// there rather than dropping inline, and the caller clears the slot
    /// two frames later when the GPU is guaranteed to be done.
    pub(crate) fn resize(
        &mut self,
        device: &wgpu::Device,
        new_size: (u32, u32),
        new_render_size: (u32, u32),
        retire_index: usize,
    ) -> i64 {
        assert!(
            new_size.0 > 0 && new_size.1 > 0,
            "MeshletView::resize requires non-zero dimensions"
        );

        let (vbuf_texture, vbuf_view) = create_2d_attachment(
            device,
            "meshlet_view_vbuf",
            new_render_size,
            VISIBILITY_BUFFER_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        let (depth_texture, depth_view) = create_2d_attachment(
            device,
            "meshlet_view_depth",
            new_render_size,
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let depth_sample_view = depth_sample_view(&depth_texture);
        let (color_texture, color_view) = create_2d_attachment(
            device,
            "meshlet_view_color",
            new_size,
            DEFERRED_COLOR_FORMAT,
            wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
        );

        // Rebuilt in lock-step with the production attachments, and only
        // when construction installed it — that caps decision is
        // preserved across resize rather than re-surveyed.
        let (triangle_density_texture, triangle_density_view) = Self::create_density(
            device,
            new_render_size,
            self.triangle_density_texture.is_some(),
        );

        let old_pyramid_bytes = self.pyramid_bytes();
        // Recreated only if they were already allocated: the lazy
        // pyramids stay lazy across a resize.
        let hiz_prev = self
            .hiz_prev
            .is_some()
            .then(|| HiZ::new(device, new_render_size.0, new_render_size.1));
        let hiz_curr = self
            .hiz_curr
            .is_some()
            .then(|| HiZ::new(device, new_render_size.0, new_render_size.1));

        self.vbuf_texture = vbuf_texture;
        self.vbuf_view = vbuf_view;
        self.depth_texture = depth_texture;
        self.depth_view = depth_view;
        self.depth_sample_view = depth_sample_view;
        self.color_texture = color_texture;
        self.color_view = color_view;
        self.triangle_density_texture = triangle_density_texture;
        self.triangle_density_view = triangle_density_view;

        if let Some(previous) = std::mem::replace(&mut self.hiz_prev, hiz_prev) {
            self.retired_pyramids[retire_index].push(previous);
        }
        if let Some(previous) = std::mem::replace(&mut self.hiz_curr, hiz_curr) {
            self.retired_pyramids[retire_index].push(previous);
        }
        // Both pyramids are fresh — they need `clear_to_far` before the
        // next pass A samples them.
        self.hi_z_initialized = false;

        // #493: keep the atomic R64 path in lockstep, or one of the two
        // vbuf paths would be valid and the other stale.
        if let Some(stage) = self.vbuf64_stage.as_mut() {
            stage.resize(device, new_render_size, new_size);
        }
        self.size = new_size;
        self.render_size = new_render_size;

        self.pyramid_bytes() as i64 - old_pyramid_bytes as i64
    }

    /// Bytes held by the two pyramids, zero while they are lazy.
    fn pyramid_bytes(&self) -> u64 {
        self.hiz_prev.as_ref().map(|p| p.byte_size()).unwrap_or(0)
            + self.hiz_curr.as_ref().map(|p| p.byte_size()).unwrap_or(0)
    }
}
