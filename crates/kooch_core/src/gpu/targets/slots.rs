//! Which slot a target request lands in, and when a released one may be freed (#392).
//!
//! 🔴 No wgpu here on purpose: the bookkeeping is what has the edge cases, and a test for it must
//! not need an adapter.

/// What makes two targets interchangeable. Anything a `wgpu::TextureDescriptor` needs that is not
/// here is the same for every render target the engine allocates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TargetDesc {
    pub size: (u32, u32),
    pub format: wgpu::TextureFormat,
    pub usage: wgpu::TextureUsages,
    pub mips: u32,
    pub samples: u32,
}

impl TargetDesc {
    /// A single-sampled, single-mip colour or depth attachment — what almost every target is.
    pub fn attachment(size: (u32, u32), format: wgpu::TextureFormat) -> Self {
        Self {
            size: (size.0.max(1), size.1.max(1)),
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            mips: 1,
            samples: 1,
        }
    }

    pub fn with_usage(mut self, usage: wgpu::TextureUsages) -> Self {
        self.usage = usage;
        self
    }

    pub fn with_mips(mut self, mips: u32) -> Self {
        self.mips = mips.max(1);
        self
    }
}

/// How many frames a released target is kept before it may be handed out again.
///
/// 🔴 Three, because Mesa radv invalidates a bind group whose texture was dropped while the GPU may
/// still be reading it. The pool reuses rather than drops, and the wait is the same wait.
pub const RETIREMENT: usize = 3;

/// One target's bookkeeping, with no texture attached.
struct Slot {
    desc: TargetDesc,
    /// `false` once released, and only then after its retirement.
    held: bool,
}

/// Slots, the free list, and the retirement ring.
#[derive(Default)]
pub(super) struct Slots {
    slots: Vec<Slot>,
    /// Indices released this frame and the two before it.
    retiring: [Vec<u32>; RETIREMENT],
    frame: usize,
}

impl Slots {
    /// A free slot matching `desc`, or a new one. The index is the caller's handle.
    pub(super) fn claim(&mut self, desc: TargetDesc) -> (u32, Fresh) {
        let free = (0..self.slots.len()).find(|&index| {
            let slot = &self.slots[index];
            !slot.held
                && slot.desc == desc
                && !self
                    .retiring
                    .iter()
                    .any(|frame| frame.contains(&(index as u32)))
        });
        if let Some(index) = free {
            self.slots[index].held = true;
            return (index as u32, Fresh::Reused);
        }
        self.slots.push(Slot { desc, held: true });
        ((self.slots.len() - 1) as u32, Fresh::Created)
    }

    /// Hands a target back. It waits out [`RETIREMENT`] frames before anything reuses it.
    pub(super) fn release(&mut self, index: u32) {
        if let Some(slot) = self.slots.get_mut(index as usize) {
            slot.held = false;
            self.retiring[self.frame].push(index);
        }
    }

    /// Ends the frame: the slots released three frames ago become reusable.
    pub(super) fn end_frame(&mut self) {
        self.frame = (self.frame + 1) % RETIREMENT;
        self.retiring[self.frame].clear();
    }

    pub(super) fn desc(&self, index: u32) -> Option<TargetDesc> {
        self.slots.get(index as usize).map(|slot| slot.desc)
    }

    pub(super) fn len(&self) -> usize {
        self.slots.len()
    }

    /// How many slots nothing holds — what a leak shows up as.
    pub(super) fn free(&self) -> usize {
        self.slots.iter().filter(|slot| !slot.held).count()
    }
}

/// Whether a claim has a texture already.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Fresh {
    Reused,
    Created,
}

#[cfg(test)]
mod tests;
