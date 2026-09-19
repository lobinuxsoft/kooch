//! The ring the raster's counters come home in.

/// The three-slot ring the raster's counters come home in.
pub struct RasterReadback {
    slots: Vec<(wgpu::Buffer, std::sync::Arc<std::sync::Mutex<SlotState>>)>,
    /// Which camera each slot's copy was taken for, captured when the copy is RECORDED. The ring is
    /// frames deep and the cameras take turns, so asking the rasterizer at map time labels the
    /// number with whichever one ran last.
    views: Vec<u32>,
    next: usize,
    pending: Option<usize>,
    slot_words: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SlotState {
    Writable,
    InFlight,
    Ready,
}

impl RasterReadback {
    pub fn new(device: &wgpu::Device, words: u32) -> Self {
        let size = words as u64 * 4;
        Self {
            slots: (0..3)
                .map(|i| {
                    (
                        device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some(&format!("page_raster_readback_{i}")),
                            size,
                            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                            mapped_at_creation: false,
                        }),
                        std::sync::Arc::new(std::sync::Mutex::new(SlotState::Writable)),
                    )
                })
                .collect(),
            views: vec![0; 3],
            next: 0,
            pending: None,
            slot_words: words as usize,
        }
    }

    /// Copies the counters into a free slot. A frame with none simply
    /// skips: the cached count is one frame older, which is the same
    /// kind of stale it already was.
    pub fn record(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        counters: &wgpu::Buffer,
        view: u32,
    ) {
        let Some(index) = self.acquire() else {
            return;
        };
        self.views[index] = view;
        encoder.copy_buffer_to_buffer(
            counters,
            0,
            &self.slots[index].0,
            0,
            self.slot_words as u64 * 4,
        );
        self.pending = Some(index);
    }

    /// Maps what was recorded and returns whatever earlier frames
    /// finished. Call once a frame, **after** the submit.
    pub fn poll(&mut self) -> Option<(Vec<u32>, u32)> {
        if let Some(index) = self.pending.take() {
            let (buffer, state) = &self.slots[index];
            *state.lock().unwrap() = SlotState::InFlight;
            let flag = state.clone();
            buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    if result.is_ok() {
                        *flag.lock().unwrap() = SlotState::Ready;
                    }
                });
        }
        for (index, (buffer, state)) in self.slots.iter().enumerate() {
            if *state.lock().unwrap() != SlotState::Ready {
                continue;
            }
            let words = {
                let mapped = buffer.slice(..).get_mapped_range();
                bytemuck::cast_slice::<u8, u32>(&mapped).to_vec()
            };
            buffer.unmap();
            *state.lock().unwrap() = SlotState::Writable;
            return Some((words, self.views[index]));
        }
        None
    }

    fn acquire(&mut self) -> Option<usize> {
        for _ in 0..self.slots.len() {
            let index = self.next;
            self.next = (self.next + 1) % self.slots.len();
            if *self.slots[index].1.lock().unwrap() == SlotState::Writable {
                return Some(index);
            }
        }
        None
    }
}
