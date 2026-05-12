//! Async CPU mirror of the cull pipeline's per-stage survivor
//! counters (#454.6).
//!
//! The cull shader atomicAdds into [`MeshletCull::stage_counters_buffer`]
//! every frame the editor selects a debug mode that flips
//! `CullParams.debug_active`. To surface the result in the editor's
//! stats overlay without stalling on `device.poll`, this module
//! mirrors the [`super::gpu_timers::MeshletGpuTimers`] state machine:
//! a 3-slot ring of `MAP_READ` buffers, async `map_async` callbacks,
//! and a `last_frame_counts` cache the renderer reads cheaply.
//!
//! Synchronous readback would force one stall per frame for a
//! 16-byte payload — measurable on Steam Deck and OneXFly. The ring
//! lets the loop write a fresh slot every frame while older slots
//! drain on the wgpu driver thread; if every slot is in flight the
//! frame skips the readback and the HUD keeps the previously-cached
//! value.
//!
//! ## State machine per slot
//!
//! ```text
//!   Writable ── encode + submit + map_async ──► InFlight
//!       ▲                                          │
//!       │                                          │  callback fires
//!       │                                          ▼
//!       └───── drain_ready: read + unmap ◄──── Ready
//! ```
//!
//! ## Activation gate
//!
//! Allocated unconditionally inside [`MeshletRenderStage::new`] —
//! the GPU buffer is 48 bytes total (3 slots × 16 B) and the ring
//! state is a few hundred bytes. The render path only schedules a
//! copy + `map_async` when the active frame's `debug_active` is
//! true; production rendering pays nothing beyond the allocation.

use std::sync::{Arc, Mutex};

const SLOT_COUNT: usize = 3;
const READBACK_BYTES: u64 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SlotState {
    Writable,
    InFlight,
    Ready,
}

struct CounterSlot {
    buffer: wgpu::Buffer,
    state: Arc<Mutex<SlotState>>,
}

/// 4-element snapshot pulled out of the cull shader's
/// `stage_counters[]` buffer.
///
/// - `[0]` after_frustum  (passed frustum)
/// - `[1]` after_backface (passed frustum + backface)
/// - `[2]` after_hi_z     (Hi-Z 2-pass entry only writes here)
/// - `[3]` total_visible  (terminal — equals visible_count)
pub type CullStageCounts = [u32; 4];

/// Owned by [`super::MeshletRenderStage`]. See module docs for the
/// state machine.
pub struct MeshletStageCounters {
    slots: Vec<CounterSlot>,
    next_write_idx: usize,
    last_frame_counts: Option<CullStageCounts>,
}

impl MeshletStageCounters {
    /// Allocates the ring of MAP_READ buffers. Cheap (48 B GPU + a
    /// few hundred B CPU); always called from
    /// [`super::MeshletRenderStage::new`].
    pub fn new(device: &wgpu::Device) -> Self {
        let slots = (0..SLOT_COUNT)
            .map(|i| {
                let label = format!("meshlet_stage_counters_readback_{i}");
                CounterSlot {
                    buffer: device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some(&label),
                        size: READBACK_BYTES,
                        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }),
                    state: Arc::new(Mutex::new(SlotState::Writable)),
                }
            })
            .collect();
        Self {
            slots,
            next_write_idx: 0,
            last_frame_counts: None,
        }
    }

    /// Most recent successfully-read counts, or `None` until the
    /// first slot completes its readback (typically 1-2 frames
    /// after the first debug-mode dispatch).
    pub fn last_frame_counts(&self) -> Option<CullStageCounts> {
        self.last_frame_counts
    }

    /// Walks every slot, reads any that fired their callback,
    /// updates [`Self::last_frame_counts`], and resets those slots
    /// to `Writable`. Cheap when nothing has fired — `Mutex::lock`
    /// + state compare per slot.
    ///
    /// Call once per frame BEFORE acquiring a new slot.
    pub fn drain_ready(&mut self) {
        for slot in &self.slots {
            let mut state_guard = slot.state.lock().unwrap();
            if *state_guard == SlotState::Ready {
                {
                    let view = slot.buffer.slice(..).get_mapped_range();
                    let words: &[u32] = bytemuck::cast_slice(&view);
                    self.last_frame_counts = Some([words[0], words[1], words[2], words[3]]);
                }
                slot.buffer.unmap();
                *state_guard = SlotState::Writable;
            }
        }
    }

    /// Returns the index of the next `Writable` slot, advancing the
    /// round-robin pointer. Returns `None` when every slot is in
    /// flight — the caller should skip the readback for this frame
    /// and rely on the cached [`Self::last_frame_counts`].
    pub fn acquire_slot(&mut self) -> Option<usize> {
        for _ in 0..self.slots.len() {
            let idx = self.next_write_idx;
            self.next_write_idx = (self.next_write_idx + 1) % self.slots.len();
            let state = *self.slots[idx].state.lock().unwrap();
            if state == SlotState::Writable {
                return Some(idx);
            }
        }
        None
    }

    /// Records a `copy_buffer_to_buffer` from the cull's
    /// `stage_counters` buffer into the ring slot at `slot_idx`.
    /// Pair with [`Self::submit_readback`] AFTER `queue.submit`.
    pub fn write_copy(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        cull_stage_counters: &wgpu::Buffer,
        slot_idx: usize,
    ) {
        encoder.copy_buffer_to_buffer(
            cull_stage_counters,
            0,
            &self.slots[slot_idx].buffer,
            0,
            READBACK_BYTES,
        );
    }

    /// Schedules the slot's buffer for async map_async. wgpu's
    /// driver thread fires the closure when the GPU has finished
    /// the copy AND the buffer is host-visible.
    pub fn submit_readback(&self, slot_idx: usize) {
        let slot = &self.slots[slot_idx];
        *slot.state.lock().unwrap() = SlotState::InFlight;
        let state = slot.state.clone();
        slot.buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                if result.is_ok() {
                    *state.lock().unwrap() = SlotState::Ready;
                }
                // Map errors are device-loss territory; leave the
                // slot InFlight so subsequent acquires skip it. The
                // cached `last_frame_counts` keeps reporting the
                // last good value instead of crashing.
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_count_invariant() {
        // The state-machine reasoning in the module doc assumes ≥ 2
        // slots so a slot is always free while another is in flight.
        // 3 gives a comfortable safety margin against driver-thread
        // callback latency. Lock the constant so a future regression
        // (someone setting it to 1 to "save memory") trips this.
        assert!(SLOT_COUNT >= 2, "ring must have at least 2 slots");
    }

    #[test]
    fn readback_size_matches_four_u32_counters() {
        // Cull shader writes 4 atomic u32 — readback must size to
        // exactly that or the bytemuck::cast_slice in drain_ready
        // panics on length mismatch.
        assert_eq!(READBACK_BYTES, 16);
        assert_eq!(READBACK_BYTES as usize, std::mem::size_of::<CullStageCounts>());
    }
}
