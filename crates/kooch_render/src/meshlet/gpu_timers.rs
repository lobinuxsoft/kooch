//! GPU frame timing via wgpu timestamp queries (#463.4, #335).
//!
//! Wraps a `QuerySet` (2N entries for N stages), a GPU-side resolve
//! buffer, and a ring of CPU-mappable readback buffers so the render
//! loop can sample wall-clock GPU time without blocking on
//! `device.poll`.
//!
//! ## Stages
//!
//! A timer instance owns `stage_count` pairs of timestamps. Stage `i`
//! occupies query slots `(2i, 2i + 1)`. Callers write start / end per
//! stage to break a frame into named segments (cull / vbuf raster /
//! deferred shade in the production pipeline; bench scenarios add
//! more granularity).
//!
//! Single-stage instances (`stage_count = 1`) preserve the original
//! pre-#335 API surface: `write_start` + `write_end_and_copy` are
//! kept as thin aliases over `write_stage_start(0)` /
//! `write_stage_end(0)` + `resolve_and_copy`, and `last_frame_ms`
//! returns the only stage's duration.
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
    stage_count: u32,
    query_count: u32,
    readback_bytes: u64,
    query_set: Option<wgpu::QuerySet>,
    resolve_buffer: Option<wgpu::Buffer>,
    /// Nanoseconds per timestamp tick, taken from
    /// `queue.get_timestamp_period()`. Drives the tick → ms math in
    /// [`Self::drain_ready`].
    timestamp_period_ns: f32,
    slots: Vec<TimerSlot>,
    next_write_idx: usize,
    /// Most recent successfully-read GPU per-stage durations, in ms.
    /// `last_stage_timings_ms[i]` is the duration of stage `i` from
    /// the most recent slot that finished its readback. Persists
    /// across frames so the HUD always has a value to display even
    /// when no slot has produced a fresh reading this frame.
    last_stage_timings_ms: Option<Vec<f32>>,
}

impl MeshletGpuTimers {
    /// Builds a live timer set with a single (start, end) pair if the
    /// adapter exposes `Features::TIMESTAMP_QUERY`; otherwise returns
    /// a no-op instance. Back-compat alias for callers that just need
    /// total frame time; see [`Self::new_with_stages`] for granular
    /// per-pass timing.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, adapter: &wgpu::Adapter) -> Self {
        Self::new_with_stages(device, queue, adapter, 1)
    }

    /// Builds a live timer set with `stage_count` pairs of timestamps
    /// — one (start, end) per pass the caller wants to time
    /// separately. Used by the mesh-frame bench (#335) to break a
    /// frame into cull / vbuf raster / deferred shade segments.
    ///
    /// Returns a no-op instance if the adapter lacks
    /// `Features::TIMESTAMP_QUERY` or
    /// `Features::TIMESTAMP_QUERY_INSIDE_ENCODERS`.
    pub fn new_with_stages(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        adapter: &wgpu::Adapter,
        stage_count: u32,
    ) -> Self {
        assert!(stage_count >= 1, "stage_count must be >= 1");
        // Two distinct features are required:
        // - TIMESTAMP_QUERY: lets us create the QuerySet itself.
        // - TIMESTAMP_QUERY_INSIDE_ENCODERS: lets us call
        //   `encoder.write_timestamp(...)` BETWEEN passes, which is
        //   what we do (start before cull, end after deferred shade,
        //   no `timestamp_writes` plumbed inside any pass descriptor).
        // Either one missing → no-op instance; the HUD reports
        // "GPU n/a" instead of crashing in queue submit.
        let f = adapter.features();
        let supported = f.contains(wgpu::Features::TIMESTAMP_QUERY)
            && f.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS);
        if !supported {
            return Self::disabled();
        }
        let query_count = stage_count * 2;
        let readback_bytes = (query_count as u64) * std::mem::size_of::<u64>() as u64;
        let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("meshlet_gpu_timers_query_set"),
            ty: wgpu::QueryType::Timestamp,
            count: query_count,
        });
        let resolve_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meshlet_gpu_timers_resolve"),
            size: readback_bytes,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let slots = (0..SLOT_COUNT)
            .map(|i| {
                let label = format!("meshlet_gpu_timers_readback_{i}");
                TimerSlot {
                    buffer: device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some(&label),
                        size: readback_bytes,
                        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }),
                    state: Arc::new(Mutex::new(SlotState::Writable)),
                }
            })
            .collect();
        Self {
            enabled: true,
            stage_count,
            query_count,
            readback_bytes,
            query_set: Some(query_set),
            resolve_buffer: Some(resolve_buffer),
            timestamp_period_ns: queue.get_timestamp_period(),
            slots,
            next_write_idx: 0,
            last_stage_timings_ms: None,
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
            stage_count: 1,
            query_count: 2,
            readback_bytes: 16,
            query_set: None,
            resolve_buffer: None,
            timestamp_period_ns: 0.0,
            slots: Vec::new(),
            next_write_idx: 0,
            last_stage_timings_ms: None,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn stage_count(&self) -> u32 {
        self.stage_count
    }

    /// Sum of every stage's duration from the most recent successful
    /// readback. Equivalent to "total GPU frame time" for callers
    /// using `stage_count = 1`. `None` until the first slot completes
    /// its readback (typically 1-2 frames after `enable_gpu_timers`
    /// is called).
    pub fn last_frame_ms(&self) -> Option<f32> {
        self.last_stage_timings_ms.as_ref().map(|v| v.iter().sum())
    }

    /// Duration of stage `stage_idx` from the most recent successful
    /// readback. Returns `None` if no readback has landed yet or
    /// `stage_idx` exceeds `stage_count`.
    pub fn last_stage_ms(&self, stage_idx: u32) -> Option<f32> {
        self.last_stage_timings_ms
            .as_ref()
            .and_then(|v| v.get(stage_idx as usize).copied())
    }

    /// Per-stage durations from the most recent successful readback,
    /// in encoder order. Length equals `stage_count`. `None` until
    /// the first slot completes its readback.
    pub fn last_frame_stage_timings(&self) -> Option<&[f32]> {
        self.last_stage_timings_ms.as_deref()
    }

    /// Walks every slot, reads timestamps from any that fired their
    /// callback, updates [`Self::last_stage_timings_ms`], and resets
    /// those slots to `Writable`. Cheap when nothing has fired —
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
                    let mut timings = Vec::with_capacity(self.stage_count as usize);
                    for stage in 0..self.stage_count as usize {
                        let start = ticks[stage * 2];
                        let end = ticks[stage * 2 + 1];
                        let delta = end.saturating_sub(start);
                        let ms = (delta as f64 * self.timestamp_period_ns as f64) / 1_000_000.0;
                        timings.push(ms as f32);
                    }
                    self.last_stage_timings_ms = Some(timings);
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

    /// Writes the START timestamp for stage `stage_idx` into the
    /// encoder. Pair with [`Self::write_stage_end`] using the same
    /// stage index. Out-of-range `stage_idx` is a no-op so a caller
    /// using `stage_count = 1` can't accidentally over-write.
    pub fn write_stage_start(&self, encoder: &mut wgpu::CommandEncoder, stage_idx: u32) {
        let Some(qs) = &self.query_set else { return };
        if stage_idx >= self.stage_count {
            return;
        }
        encoder.write_timestamp(qs, stage_idx * 2);
    }

    /// Writes the END timestamp for stage `stage_idx`.
    pub fn write_stage_end(&self, encoder: &mut wgpu::CommandEncoder, stage_idx: u32) {
        let Some(qs) = &self.query_set else { return };
        if stage_idx >= self.stage_count {
            return;
        }
        encoder.write_timestamp(qs, stage_idx * 2 + 1);
    }

    /// Resolves every timestamp into the resolve buffer and copies
    /// the result into `slot_idx`'s readback buffer. Call once per
    /// frame AFTER every `write_stage_end` and BEFORE
    /// [`Self::submit_readback`].
    pub fn resolve_and_copy(&self, encoder: &mut wgpu::CommandEncoder, slot_idx: usize) {
        let (Some(qs), Some(resolve)) = (&self.query_set, &self.resolve_buffer) else {
            return;
        };
        encoder.resolve_query_set(qs, 0..self.query_count, resolve, 0);
        encoder.copy_buffer_to_buffer(
            resolve,
            0,
            &self.slots[slot_idx].buffer,
            0,
            self.readback_bytes,
        );
    }

    /// Back-compat alias for `stage_count = 1`: writes the single
    /// stage's start timestamp. New callers should prefer
    /// [`Self::write_stage_start`] for clarity.
    pub fn write_start(&self, encoder: &mut wgpu::CommandEncoder) {
        self.write_stage_start(encoder, 0);
    }

    /// Back-compat alias for `stage_count = 1`: writes the single
    /// stage's end timestamp, resolves, and copies. Equivalent to
    /// `write_stage_end(encoder, 0)` + `resolve_and_copy(encoder,
    /// slot_idx)`.
    pub fn write_end_and_copy(&self, encoder: &mut wgpu::CommandEncoder, slot_idx: usize) {
        self.write_stage_end(encoder, 0);
        self.resolve_and_copy(encoder, slot_idx);
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
        assert_eq!(timers.last_stage_ms(0), None);
        assert!(timers.last_frame_stage_timings().is_none());
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
    fn disabled_instance_reports_stage_count_one() {
        // Disabled timers still answer `stage_count()` so callers
        // using the multi-stage API can branch on `is_enabled()`
        // without first probing the count.
        let timers = MeshletGpuTimers::disabled();
        assert_eq!(timers.stage_count(), 1);
    }
}
