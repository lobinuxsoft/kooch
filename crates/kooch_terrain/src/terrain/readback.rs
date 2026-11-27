//! The batch's readback: each block's counts and cuts, then the pass timestamps.

use super::{BatchMean, COUNT_WORDS, Cuts, GpuTimes};

pub(super) struct Readback<'a> {
    pub bytes: &'a [u8],
    /// Where the timestamps start, after every block's counts.
    pub stamps: u64,
}

impl Readback<'_> {
    fn word(&self, at: u64) -> Option<u32> {
        let at = usize::try_from(at).ok()?;
        let raw = self.bytes.get(at..at + 4)?;
        Some(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
    }

    /// Block `b`'s counts: vertices, meshlets, meshlet vertices, triangle bytes.
    pub fn counts(&self, b: usize) -> Option<[u32; 4]> {
        let entry = b as u64 * COUNT_WORDS * 4;
        Some([
            self.word(entry)?,
            self.word(entry + 4)?,
            self.word(entry + 8)?,
            self.word(entry + 12)?,
        ])
    }

    pub fn cuts(&self, b: usize) -> Option<Cuts> {
        let entry = b as u64 * COUNT_WORDS * 4 + 16;
        let mut cuts = [0; 16];
        for (k, cut) in cuts.iter_mut().enumerate() {
            *cut = self.word(entry + k as u64 * 4)?;
        }
        Some(cuts)
    }

    pub fn times(&self, period_ns: f32, blocks: u32) -> GpuTimes {
        let stamp = |k: u64| {
            let lo = self.word(self.stamps + k * 8).unwrap_or(0);
            let hi = self.word(self.stamps + k * 8 + 4).unwrap_or(0);
            u64::from(lo) | (u64::from(hi) << 32)
        };
        let ms = |from: u64, to: u64| (to.saturating_sub(from)) as f32 * period_ns / 1e6;
        GpuTimes {
            generate_ms: ms(stamp(0), stamp(1)),
            mesh_ms: ms(stamp(2), stamp(3)),
            write_ms: ms(stamp(4), stamp(5)),
            blocks,
            mean: BatchMean::default(),
        }
    }
}
