//! The three-slot ring the marking's counters come home in.

use super::*;

/// The three-slot ring the counters come home in.
pub(super) struct Readback {
    slots: Vec<(wgpu::Buffer, Arc<Mutex<SlotState>>)>,
    /// What each slot's dispatch was: its render size, its camera and the pool slice it allocated
    /// from.
    labels: Vec<Label>,
    next: usize,
}

#[derive(Clone, Copy, Default)]
pub(super) struct Label {
    pub(super) size: (u32, u32),
    pub(super) view: u32,
    pub(super) capacity: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SlotState {
    Writable,
    InFlight,
    Ready,
}

impl Readback {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let slots = (0..3)
            .map(|i| {
                (
                    device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some(&format!("page_mark_readback_{i}")),
                        size: COUNTERS * 4,
                        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }),
                    Arc::new(Mutex::new(SlotState::Writable)),
                )
            })
            .collect();
        Self {
            slots,
            labels: vec![Label::default(); 3],
            next: 0,
        }
    }

    /// Copies the counters into a free slot, if there is one.
    pub(super) fn record(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        counters: &wgpu::Buffer,
        label: Label,
    ) -> Option<usize> {
        let index = self.acquire()?;
        encoder.copy_buffer_to_buffer(counters, 0, &self.slots[index].0, 0, COUNTERS * 4);
        self.labels[index] = label;
        Some(index)
    }

    /// Asks wgpu to map the slot. Call **after** the encoder carrying
    /// the copy has been submitted.
    pub(super) fn submit(&self, index: usize) {
        let (buffer, state) = &self.slots[index];
        *state.lock().unwrap() = SlotState::InFlight;
        let flag = state.clone();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                if result.is_ok() {
                    *flag.lock().unwrap() = SlotState::Ready;
                }
                // A map error is device-loss territory. Leaving the slot
                // in flight means later frames skip it rather than
                // panicking on wgpu's driver thread.
            });
    }

    pub(super) fn acquire(&mut self) -> Option<usize> {
        for _ in 0..self.slots.len() {
            let index = self.next;
            self.next = (self.next + 1) % self.slots.len();
            if *self.slots[index].1.lock().unwrap() == SlotState::Writable {
                return Some(index);
            }
        }
        None
    }

    pub(super) fn take(&mut self) -> Option<MarkCounts> {
        for (index, (buffer, state)) in self.slots.iter().enumerate() {
            if *state.lock().unwrap() != SlotState::Ready {
                continue;
            }
            let Label {
                size,
                view,
                capacity,
            } = self.labels[index];
            let counts = {
                let mapped = buffer.slice(..).get_mapped_range();
                let words: &[u32] = bytemuck::cast_slice(&mapped);
                MarkCounts {
                    resident: words[0],
                    samples: words[1],
                    pairs: words[2],
                    overflow: words[3],
                    culled: words[6],
                    distant: words[24],
                    froxels: words[9],
                    peak_lights: words[16],
                    by_froxel: false,
                    pool: PoolCounts {
                        claims: words[8],
                        overflow: words[5],
                        reused: words[7],
                        leaked: words[10],
                        alive: words[11],
                        evicted: words[12],
                        denied: words[13],
                        preempted: words[14],
                        cutoff: words[15],
                        high: words[17],
                        free: words[18],
                        demand: words[19],
                        popped: words[20],
                        bumped: words[21],
                        pushed: words[22],
                        empty: words[23],
                        bias_local: words[4] & 0xff,
                        bias_sun: words[4] >> 8,
                        capacity,
                    },
                    size,
                    view,
                }
            };
            buffer.unmap();
            *state.lock().unwrap() = SlotState::Writable;
            return Some(counts);
        }
        None
    }
}
