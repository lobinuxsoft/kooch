//! Free-list bookkeeping for the [`GlobalMeshPool`](super::GlobalMeshPool): where each registered
//! mesh sits in the concatenated arrays, and which ranges a released mesh left behind.

/// A range of slots in one of the pool's arrays.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub(super) struct Span {
    pub start: u32,
    pub len: u32,
}

impl Span {
    pub fn end(&self) -> u32 {
        self.start + self.len
    }

    pub fn range(&self) -> std::ops::Range<usize> {
        self.start as usize..self.end() as usize
    }
}

/// Where one mesh sits in each array.
///
/// 🔴 CPU-only, and the reason the free list needs it: once the pool has holes, a mesh's extent
/// can no longer be read off the next descriptor's offset.
#[derive(Copy, Clone, Debug, Default)]
pub(super) struct Slot {
    pub vertices: Span,
    pub meshlet_vertices: Span,
    pub triangles: Span,
    pub meshlets: Span,
    pub groups: Span,
    pub live: bool,
}

/// First-fit allocator over one array's holes.
///
/// `align` keeps the triangle byte array on u32 boundaries: the cull shader reads `array<u32>` and
/// extracts bytes by shift-mask, so an unaligned base offset mis-reads the first byte.
#[derive(Debug, Default)]
pub(super) struct Holes {
    free: Vec<Span>,
    align: u32,
}

impl Holes {
    pub fn aligned(align: u32) -> Self {
        Self {
            free: Vec::new(),
            align: align.max(1),
        }
    }

    /// `len` slots out of the holes, or `None` when the caller has to grow the array instead.
    pub fn take(&mut self, len: u32) -> Option<Span> {
        let len = self.rounded(len);
        if len == 0 {
            return Some(Span::default());
        }
        let at = self.free.iter().position(|hole| hole.len >= len)?;
        let hole = self.free[at];
        match hole.len == len {
            true => {
                self.free.remove(at);
            }
            false => {
                self.free[at] = Span {
                    start: hole.start + len,
                    len: hole.len - len,
                }
            }
        }
        Some(Span {
            start: hole.start,
            len,
        })
    }

    /// Returns `span`, coalesced with whatever it now touches — without the merge, a pool edited
    /// long enough is all holes and none of them fit anything.
    pub fn give(&mut self, span: Span) {
        if span.len == 0 {
            return;
        }
        self.free.push(span);
        self.free.sort_unstable_by_key(|hole| hole.start);
        let mut merged: Vec<Span> = Vec::with_capacity(self.free.len());
        for hole in self.free.drain(..) {
            match merged.last_mut() {
                Some(last) if last.end() == hole.start => last.len += hole.len,
                _ => merged.push(hole),
            }
        }
        self.free = merged;
    }

    /// `len` rounded up to the alignment, which is the size actually reserved.
    pub fn rounded(&self, len: u32) -> u32 {
        len.next_multiple_of(self.align)
    }

    /// Slots the holes are holding — what a fragmented pool is wasting.
    pub fn free_slots(&self) -> u64 {
        self.free.iter().map(|hole| hole.len as u64).sum()
    }
}

/// The slots of one array written since the last GPU sync, as a single range.
///
/// One range and not a list: an edit touches one mesh's ranges, and spanning the gap between two
/// distant edits still writes far less than re-uploading the array.
#[derive(Copy, Clone, Debug, Default)]
pub(super) struct Touched(Option<(u32, u32)>);

impl Touched {
    pub fn mark(&mut self, span: Span) {
        if span.len == 0 {
            return;
        }
        self.0 = Some(match self.0 {
            Some((start, end)) => (start.min(span.start), end.max(span.end())),
            None => (span.start, span.end()),
        });
    }

    /// Everything, for when the array was rebuilt rather than edited.
    pub fn all(len: usize) -> Self {
        Self::default().marked(Span {
            start: 0,
            len: len as u32,
        })
    }

    fn marked(mut self, span: Span) -> Self {
        self.mark(span);
        self
    }

    pub fn take(&mut self) -> Option<std::ops::Range<usize>> {
        self.0
            .take()
            .map(|(start, end)| start as usize..end as usize)
    }
}

/// What each of the pool's arrays owes the GPU.
#[derive(Copy, Clone, Debug, Default)]
pub(super) struct Dirty {
    pub vertices: Touched,
    pub meshlet_vertices: Touched,
    pub triangles: Touched,
    pub meshlets: Touched,
    pub descriptors: Touched,
}
