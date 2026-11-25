//! #1999 debug: the write pass's skipped jobs, counted on the GPU and read back without waiting.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

/// Interior, seam, overlap; then vertices, meshlets, meshlet vertices, bytes off; then overflow.
pub type SkipCounts = [u32; 8];

const IDLE: u8 = 0;
const MAPPING: u8 = 1;
const MAPPED: u8 = 2;

const BYTES: u64 = size_of::<SkipCounts>() as u64;

pub(super) struct Skips {
    pub total: wgpu::Buffer,
    staging: wgpu::Buffer,
    state: Arc<AtomicU8>,
    last: SkipCounts,
}

impl Skips {
    pub fn new(device: &wgpu::Device) -> Self {
        let buffer = |label, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: BYTES,
                usage,
                mapped_at_creation: false,
            })
        };
        Self {
            total: buffer(
                "kooch_planet::mesh::skips",
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            ),
            staging: buffer(
                "kooch_planet::mesh::skips_read",
                wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            ),
            state: Arc::new(AtomicU8::new(IDLE)),
            last: SkipCounts::default(),
        }
    }

    /// Copies the totals out when no read is in flight; [`Self::mapped`] after the submit.
    pub fn record(&self, encoder: &mut wgpu::CommandEncoder) -> bool {
        let idle = self.state.load(Ordering::Acquire) == IDLE;
        if idle {
            encoder.copy_buffer_to_buffer(&self.total, 0, &self.staging, 0, BYTES);
        }
        idle
    }

    pub fn mapped(&self) {
        self.state.store(MAPPING, Ordering::Release);
        let state = self.state.clone();
        self.staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                state.store(
                    if result.is_ok() { MAPPED } else { IDLE },
                    Ordering::Release,
                );
            });
    }

    /// The totals as of the last read that landed.
    pub fn totals(&mut self) -> SkipCounts {
        if self.state.load(Ordering::Acquire) == MAPPED {
            if let Ok(view) = self.staging.slice(..).get_mapped_range() {
                let words: &[u32] = bytemuck::cast_slice(&view);
                for (k, w) in self.last.iter_mut().zip(words) {
                    *k = *w;
                }
            }
            self.staging.unmap();
            self.state.store(IDLE, Ordering::Release);
        }
        self.last
    }
}
