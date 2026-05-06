//! GPU frame timing via wgpu timestamp queries (#463.4).
//!
//! Wraps a `QuerySet` (start + end), a GPU-side resolve buffer, and a
//! ring of CPU-mappable readback buffers so the render loop can
//! sample wall-clock GPU time without blocking on `device.poll`.
//!
//! ## Why a ring of N slots
//!
//! `map_async` is asynchronous: the result of frame N is observable
//! after the GPU finishes the submission AND wgpu fires the callback,
//! which typically lands one or two frames later. A single buffer
//! would force the render loop to either block until the callback
//! fires (defeating the point of async) or skip timing entirely.
//!
//! The ring lets the loop write to a fresh slot every frame while
//! older slots drain in the background. Three slots is plenty: at
//! 60 Hz with a 16 ms frame budget, the GPU finishes long before
//! we cycle back to the same slot.
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
//! `acquire_slot` walks the ring looking for a `Writable` entry; if
//! every slot is in flight (frames being submitted faster than the
//! GPU drains), it returns `None` and the caller skips timing for
//! that frame — the previously-sampled `last_frame_ms` stays valid.
//!
//! ## Feature gating
//!
//! Adapters without `Features::TIMESTAMP_QUERY` get a no-op instance
//! (`enabled == false`). Every method is a cheap branch in that case;
//! `last_frame_ms()` returns `None`. The render loop calls the same
//! API regardless — no `cfg!` clutter at the call site.

use std::sync::{Arc, Mutex};

const SLOT_COUNT: usize = 3;
const QUERY_COUNT: u32 = 2;
const READBACK_BYTES: u64 = (QUERY_COUNT as u64) * std::mem::size_of::<u64>() as u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SlotState {
    Writable,
    InFlight,
    Ready,
}

struct TimerSlot {
    buffer: wgpu::Buffer,
    state: Arc<Mutex<SlotState>>,
}

/// Owned by [`super::MeshletRenderStage`] when the editor / game
/// runtime calls `enable_gpu_timers`. See module docs for the state
/// machine.
pub struct MeshletGpuTimers {
    enabled: bool,
    query_set: Option<wgpu::QuerySet>,
    resolve_buffer: Option<wgpu::Buffer>,
    /// Nanoseconds per timestamp tick, taken from
    /// `queue.get_timestamp_period()`. Drives the tick → ms math in
    /// [`Self::drain_ready`].
    timestamp_period_ns: f32,
    slots: Vec<TimerSlot>,
    next_write_idx: usize,
    /// Most recent successfully-read GPU frame time. Persists across
    /// frames so the HUD always has a value to display even when no
    /// slot has produced a fresh reading this frame.
    last_frame_ms: Option<f32>,
}

impl MeshletGpuTimers {
    /// Builds a live timer set if the adapter exposes
    /// `Features::TIMESTAMP_QUERY`; otherwise returns a no-op
    /// instance. Caller passes `device + queue + adapter` from the
    /// engine's [`GpuContext`](ome_core::gpu::GpuContext).
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, adapter: &wgpu::Adapter) -> Self {
        if !adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return Self::disabled();
        }
        let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("meshlet_gpu_timers_query_set"),
            ty: wgpu::QueryType::Timestamp,
            count: QUERY_COUNT,
        });
        let resolve_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meshlet_gpu_timers_resolve"),
            size: READBACK_BYTES,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let slots = (0..SLOT_COUNT)
            .map(|i| {
                let label = format!("meshlet_gpu_timers_readback_{i}");
                TimerSlot {
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
            enabled: true,
            query_set: Some(query_set),
            resolve_buffer: Some(resolve_buffer),
            timestamp_period_ns: queue.get_timestamp_period(),
            slots,
            next_write_idx: 0,
            last_frame_ms: None,
        }
    }

    /// Creates a no-op instance — used as the default field value
    /// for [`super::MeshletRenderStage`] before `enable_gpu_timers`
    /// runs (or when running headless tests).
    pub fn new_disabled_for_default() -> Self {
        Self::disabled()
    }

    fn disabled() -> Self {
        Self {
            enabled: false,
            query_set: None,
            resolve_buffer: None,
            timestamp_period_ns: 0.0,
            slots: Vec::new(),
            next_write_idx: 0,
            last_frame_ms: None,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// The most recent GPU frame time read from any slot, in ms.
    /// `None` until the first slot completes its readback (typically
    /// 1-2 frames after `enable_gpu_timers` is called).
    pub fn last_frame_ms(&self) -> Option<f32> {
        self.last_frame_ms
    }

    /// Walks every slot, reads timestamps from any that fired their
    /// callback, updates [`Self::last_frame_ms`], and resets those
    /// slots to `Writable`. Cheap when nothing has fired —
    /// `Mutex::lock` + state compare per slot.
    ///
    /// Call this once per frame BEFORE acquiring a new slot for the
    /// upcoming submission.
    pub fn drain_ready(&mut self) {
        if !self.enabled {
            return;
        }
        for slot in &self.slots {
            let mut state_guard = slot.state.lock().unwrap();
            if *state_guard == SlotState::Ready {
                {
                    let view = slot.buffer.slice(..).get_mapped_range();
                    let ticks: &[u64] = bytemuck::cast_slice(&view);
                    let delta = ticks[1].saturating_sub(ticks[0]);
                    let ms =
                        (delta as f64 * self.timestamp_period_ns as f64) / 1_000_000.0;
                    self.last_frame_ms = Some(ms as f32);
                }
                slot.buffer.unmap();
                *state_guard = SlotState::Writable;
            }
        }
    }

    /// Returns the index of the next `Writable` slot, advancing the
    /// round-robin pointer. Returns `None` when every slot is in
    /// flight — the caller should skip timestamp writes for this
    /// frame and rely on the persisted [`Self::last_frame_ms`].
    pub fn acquire_slot(&mut self) -> Option<usize> {
        if !self.enabled {
            return None;
        }
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

    /// Writes the START timestamp into the encoder. Pair with
    /// [`Self::write_end_and_copy`] using the same slot index.
    pub fn write_start(&self, encoder: &mut wgpu::CommandEncoder) {
        if let Some(qs) = &self.query_set {
            encoder.write_timestamp(qs, 0);
        }
    }

    /// Writes the END timestamp, resolves both timestamps to the GPU
    /// resolve buffer, and copies the result into the slot's
    /// readback buffer. The submit/readback pair must call
    /// [`Self::submit_readback`] AFTER `queue.submit`.
    pub fn write_end_and_copy(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        slot_idx: usize,
    ) {
        let (Some(qs), Some(resolve)) = (&self.query_set, &self.resolve_buffer) else {
            return;
        };
        encoder.write_timestamp(qs, 1);
        encoder.resolve_query_set(qs, 0..QUERY_COUNT, resolve, 0);
        encoder.copy_buffer_to_buffer(
            resolve,
            0,
            &self.slots[slot_idx].buffer,
            0,
            READBACK_BYTES,
        );
    }

    /// Schedules the slot's buffer for async readback. wgpu's
    /// internal driver thread fires the closure when the GPU has
    /// finished the copy AND the buffer is host-visible. Call this
    /// AFTER `queue.submit` so the submission order matches the
    /// callback chain.
    pub fn submit_readback(&self, slot_idx: usize) {
        if !self.enabled {
            return;
        }
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
                // rest of the timer keeps reporting the persisted
                // `last_frame_ms` instead of crashing.
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_instance_acquires_no_slot_and_reports_none() {
        let timers = MeshletGpuTimers::disabled();
        assert!(!timers.is_enabled());
        assert_eq!(timers.last_frame_ms(), None);
        // Even after calling drain_ready, last_frame_ms stays None
        // because no slot ever transitions to Ready on a disabled
        // instance.
        let mut timers = timers;
        timers.drain_ready();
        assert_eq!(timers.acquire_slot(), None);
        assert_eq!(timers.last_frame_ms(), None);
    }

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
    fn readback_size_matches_two_u64_timestamps() {
        // wgpu writes one u64 per timestamp query; the resolve +
        // readback buffers must be sized for both.
        assert_eq!(READBACK_BYTES, 16);
    }
}
