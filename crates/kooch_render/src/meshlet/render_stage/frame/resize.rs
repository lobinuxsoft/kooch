//! [`MeshletRenderStage::resize_view`] — hand a view its new size.
//!
//! Everything that changes with the destination surface lives in
//! [`MeshletView`](super::super::view_targets::MeshletView);
//! the cull pipelines, rasterizer, deferred pass and mesh pool are shared
//! and unaffected. What is left here is the part the stage owns: which
//! frame slot retired pyramids park in, and the engine-wide VRAM tracker.

use super::super::{MeshletRenderStage, ViewId};

impl MeshletRenderStage {
    /// Recreates the primary view's attachments at `new_size` if it
    /// differs from its current size.
    pub fn resize(&mut self, device: &wgpu::Device, new_size: (u32, u32)) {
        self.resize_view(self.primary, device, new_size);
    }

    /// Recreates `id`'s attachments at `new_size` if it differs from that
    /// view's current size. Stale handles are ignored — a panel that
    /// closed mid-frame should not panic the renderer.
    ///
    /// Sizes are per view, so resizing one leaves the others alone: an
    /// editor dragging the Game panel's divider must not reallocate the
    /// View panel's attachments.
    pub fn resize_view(&mut self, id: ViewId, device: &wgpu::Device, new_size: (u32, u32)) {
        let Some(view) = self.views.get_mut(id) else {
            return;
        };
        if new_size == view.size {
            return;
        }

        // Retired pyramids park in the current frame slot rather than
        // dropping inline: the next render rotates the index and clears
        // the slot that is two frames old, by which point the GPU is
        // done with them. Mesa radv invalidates bind groups dropped
        // while still in flight.
        let pyramid_delta = view.resize(device, new_size, self.frame_bind_groups_index);

        if let Some(tracker) = &self.vram_tracker {
            // A resize can free more than it allocates — shrinking the
            // panel makes both pyramids smaller — so the delta is
            // signed and each direction goes to its own call.
            if pyramid_delta > 0 {
                tracker.add(pyramid_delta as u64);
            } else if pyramid_delta < 0 {
                tracker.sub(pyramid_delta.unsigned_abs());
            }
        }
    }
}
