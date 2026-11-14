//! Large rasters and videos (#1893), packed into a texture that lives on the GPU only: each one's
//! texels are handed to the renderer once and dropped, so no process keeps a CPU copy.

use etagere::{AllocId, Allocation, BucketedAtlasAllocator, size2};
use kooch_ecs::AtlasUpload;

use super::MAX_SIDE;

/// A raster whose longer side passes this is packed in the pages. 🔴 In the glyph atlas a 1080p
/// background forced 4096², 64 MB mirrored on the host, the wire and the editor.
pub(crate) const PAGE_ABOVE: u32 = 512;

#[derive(Default)]
pub(super) struct Pages {
    allocator: Option<BucketedAtlasAllocator>,
    side: u32,
    uploads: Vec<AtlasUpload>,
}

impl Pages {
    pub(super) fn side(&self) -> u32 {
        self.side
    }

    /// A slot, doubling the pages while they are under [`MAX_SIDE`]; `None` once they are full.
    pub(super) fn allocate(&mut self, w: u32, h: u32) -> Option<Allocation> {
        if w > MAX_SIDE || h > MAX_SIDE {
            return None;
        }
        if self.allocator.is_none() {
            self.side = w.max(h).next_power_of_two().max(PAGE_ABOVE);
        }
        let side = self.side as i32;
        let allocator = self
            .allocator
            .get_or_insert_with(|| BucketedAtlasAllocator::new(size2(side, side)));
        let size = size2(w as i32, h as i32);
        loop {
            if let Some(slot) = allocator.allocate(size) {
                return Some(slot);
            }
            if self.side >= MAX_SIDE {
                return None;
            }
            self.side *= 2;
            allocator.grow(size2(self.side as i32, self.side as i32));
        }
    }

    pub(super) fn deallocate(&mut self, id: AllocId) {
        if let Some(allocator) = &mut self.allocator {
            allocator.deallocate(id);
        }
    }

    /// Queues `rgba` for the slot at `[x0, y0]`, its edge repeated one texel out as the glyph
    /// atlas does it.
    pub(super) fn write(&mut self, [x0, y0]: [u32; 2], (w, h): (u32, u32), rgba: &[u8]) {
        let Some(texels) = bordered(rgba, w, h) else {
            return;
        };
        self.uploads.push(AtlasUpload {
            rect: [x0, y0, x0 + w + 2, y0 + h + 2],
            texels,
        });
    }

    pub(super) fn take_uploads(&mut self) -> Vec<AtlasUpload> {
        std::mem::take(&mut self.uploads)
    }

    /// Frees every slot; the side stays, so the renderer's texture is reused.
    pub(super) fn clear(&mut self) {
        if let Some(allocator) = &mut self.allocator {
            allocator.clear();
        }
        self.uploads.clear();
    }
}

fn bordered(rgba: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    let row = w as usize * 4;
    if w == 0 || h == 0 || rgba.len() < row * h as usize {
        return None;
    }
    let mut out = Vec::with_capacity((row + 8) * (h as usize + 2));
    for y in 0..h + 2 {
        let from = y.saturating_sub(1).min(h - 1) as usize * row;
        let line = rgba.get(from..from + row)?;
        out.extend_from_slice(line.get(..4)?);
        out.extend_from_slice(line);
        out.extend_from_slice(line.get(row - 4..)?);
    }
    Some(out)
}
