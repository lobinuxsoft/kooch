//! [`MeshletRenderStage::resize`] — hand the view its new size.
//!
//! Everything that changes with the destination surface lives in
//! [`MeshletView`](super::super::view_targets::MeshletView);
//! the cull, rasterizer, deferred pass and mesh pool are shared and
//! unaffected. What is left here is the part the stage owns: which frame
//! slot retired pyramids park in, and the engine-wide VRAM tracker.

use super::super::MeshletRenderStage;

impl MeshletRenderStage {
    /// Recreates the view's attachments at `new_size` if it differs from
    /// the current size.
    pub fn resize(&mut self, device: &wgpu::Device, new_size: (u32, u32)) {
        if new_size == self.view.size {
            return;
        }

        // Retired pyramids park in the current frame slot rather than
        // dropping inline: the next render rotates the index and clears
        // the slot that is two frames old, by which point the GPU is
        // done with them. Mesa radv invalidates bind groups dropped
        // while still in flight.
        let pyramid_delta = self
            .view
            .resize(device, new_size, self.frame_bind_groups_index);

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
