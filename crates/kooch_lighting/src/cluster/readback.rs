//! How the CPU learns how big the grid's buffers had to be.
//!
//! The index list's length is a property of how the scene is lit — how
//! many cells each light reaches — which only the GPU knows. Reading it
//! back synchronously would stall the frame for twenty-four bytes.
//!
//! So this is a ring of three mappable buffers, the same state machine
//! `MeshletStageCounters` uses: a frame copies the draw record into a
//! free slot and asks for it asynchronously; a later frame picks up
//! whatever arrived. The number is a frame or two old, and that is
//! exactly what it is for — the buffer it sizes is next frame's.
//!
//! ```text
//!   Writable ── copy + map_async ──► InFlight ── callback ──► Ready
//!       ▲                                                       │
//!       └──────────────── read + unmap ◄─────────────────────── ┘
//! ```

use std::sync::{Arc, Mutex};

use super::buffers::ClusterDraw;

const SLOT_COUNT: usize = 3;
const BYTES: u64 = std::mem::size_of::<ClusterDraw>() as u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SlotState {
    Writable,
    InFlight,
    Ready,
}

struct Slot {
    buffer: wgpu::Buffer,
    state: Arc<Mutex<SlotState>>,
}

/// The ring, plus the last record that came back.
pub(super) struct ClusterReadback {
    slots: Vec<Slot>,
    next: usize,
    last: Option<ClusterDraw>,
}

impl ClusterReadback {
    pub fn new(device: &wgpu::Device) -> Self {
        let slots = (0..SLOT_COUNT)
            .map(|i| Slot {
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(&format!("cluster_readback_{i}")),
                    size: BYTES,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                state: Arc::new(Mutex::new(SlotState::Writable)),
            })
            .collect();
        Self {
            slots,
            next: 0,
            last: None,
        }
    }

    /// The most recent record the GPU returned, or `None` until the
    /// first one lands.
    pub fn last(&self) -> Option<ClusterDraw> {
        self.last
    }

    /// Reads whatever arrived since the last call. Cheap when nothing
    /// has: a lock and a compare per slot.
    pub fn drain_ready(&mut self) {
        for slot in &self.slots {
            let mut state = slot.state.lock().unwrap();
            if *state != SlotState::Ready {
                continue;
            }
            {
                let view = slot.buffer.slice(..).get_mapped_range();
                self.last = Some(*bytemuck::from_bytes::<ClusterDraw>(&view));
            }
            slot.buffer.unmap();
            *state = SlotState::Writable;
        }
    }

    /// Copies the draw record into a free slot, if there is one.
    ///
    /// `None` back from the acquire means every slot is still in flight,
    /// and the frame simply skips the readback — the cached record is
    /// one more frame old, which is the same kind of stale it already
    /// was.
    pub fn record_copy(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        draw: &wgpu::Buffer,
    ) -> Option<usize> {
        let slot = self.acquire()?;
        encoder.copy_buffer_to_buffer(draw, 0, &self.slots[slot].buffer, 0, BYTES);
        Some(slot)
    }

    /// Asks wgpu to map the slot. Call **after** the encoder carrying
    /// the copy has been submitted.
    pub fn submit(&self, slot: usize) {
        let slot = &self.slots[slot];
        *slot.state.lock().unwrap() = SlotState::InFlight;
        let state = slot.state.clone();
        slot.buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                if result.is_ok() {
                    *state.lock().unwrap() = SlotState::Ready;
                }
                // A map error is device-loss territory. Leaving the slot
                // in flight means later frames skip it and keep using
                // the cached record, rather than panicking in a callback
                // on the wgpu driver thread.
            });
    }

    fn acquire(&mut self) -> Option<usize> {
        for _ in 0..self.slots.len() {
            let idx = self.next;
            self.next = (self.next + 1) % self.slots.len();
            if *self.slots[idx].state.lock().unwrap() == SlotState::Writable {
                return Some(idx);
            }
        }
        None
    }
}
