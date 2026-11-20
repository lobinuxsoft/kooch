//! The mean GPU cost of the last batches (#1935), steadier than one batch.

use crate::GpuTimes;

/// Batches the mean in [`GpuTimes`] spans.
pub const RECENT: usize = 32;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BatchMean {
    pub generate_ms: f32,
    pub mesh_ms: f32,
    pub write_ms: f32,
    pub blocks: f32,
    /// How many batches it spans, up to [`RECENT`].
    pub batches: u32,
}

/// The last [`RECENT`] batches, oldest overwritten.
#[derive(Default)]
pub(crate) struct Recent {
    batches: Vec<GpuTimes>,
    next: usize,
}

impl Recent {
    pub fn push(&mut self, times: GpuTimes) -> BatchMean {
        if self.batches.len() < RECENT {
            self.batches.push(times);
        } else if let Some(old) = self.batches.get_mut(self.next) {
            *old = times;
        }
        self.next = (self.next + 1) % RECENT;
        let n = self.batches.len() as f32;
        let sum = |f: fn(&GpuTimes) -> f32| self.batches.iter().map(f).sum::<f32>() / n;
        BatchMean {
            generate_ms: sum(|t| t.generate_ms),
            mesh_ms: sum(|t| t.mesh_ms),
            write_ms: sum(|t| t.write_ms),
            blocks: sum(|t| t.blocks as f32),
            batches: self.batches.len() as u32,
        }
    }
}
