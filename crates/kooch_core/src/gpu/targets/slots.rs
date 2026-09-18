//! Which slot a target request lands in (#392).
//!
//! 🔴 No wgpu here on purpose: the bookkeeping is what has the edge cases, and a test for it must
//! not need an adapter.

pub use kooch_plugin_render::{TargetDesc, TargetId};

/// One target's bookkeeping, with no texture attached.
struct Slot {
    desc: TargetDesc,
    held: bool,
}

/// Slots and which of them are held.
#[derive(Default)]
pub(super) struct Slots {
    slots: Vec<Slot>,
}

impl Slots {
    /// A free slot matching `desc`, or a new one. The index is the caller's handle.
    ///
    /// 🔴 A released slot is reusable at once. The pool never destroys a texture, and reusing one
    /// is ordered by the queue like any other write — the three-frame wait radv needs is for a
    /// texture *dropped* in flight. Waiting here held four targets where one does (#1201).
    pub(super) fn claim(&mut self, desc: TargetDesc) -> (u32, Fresh) {
        let free = self
            .slots
            .iter()
            .position(|slot| !slot.held && slot.desc == desc);
        if let Some(index) = free {
            self.slots[index].held = true;
            return (index as u32, Fresh::Reused);
        }
        self.slots.push(Slot { desc, held: true });
        ((self.slots.len() - 1) as u32, Fresh::Created)
    }

    pub(super) fn release(&mut self, index: u32) {
        if let Some(slot) = self.slots.get_mut(index as usize) {
            slot.held = false;
        }
    }

    pub(super) fn desc(&self, index: u32) -> Option<TargetDesc> {
        self.slots.get(index as usize).map(|slot| slot.desc)
    }

    pub(super) fn len(&self) -> usize {
        self.slots.len()
    }

    /// How many slots nothing holds.
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
