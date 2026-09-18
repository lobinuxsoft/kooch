//! Which slot a target request lands in (#392).
//!
//! 🔴 No wgpu here on purpose: the bookkeeping is what has the edge cases, and a test for it must
//! not need an adapter.

pub use kooch_plugin_render::{TargetDesc, TargetId};

/// Frames a free slot waits for another claim before its texture goes. Past the swapchain's two
/// frames in flight, so nothing still reads a texture when it is dropped (radv invalidates one
/// dropped in flight).
pub(super) const KEEP_FRAMES: u64 = 3;

/// One target's bookkeeping, with no texture attached.
struct Slot {
    desc: TargetDesc,
    held: bool,
    /// Whether it has a texture. An evicted slot keeps its index for the next new target.
    alive: bool,
    last_used: u64,
}

/// Slots and which of them are held.
#[derive(Default)]
pub(super) struct Slots {
    slots: Vec<Slot>,
    frame: u64,
}

impl Slots {
    /// A free slot matching `desc`, or a new one. The index is the caller's handle.
    ///
    /// 🔴 A released slot is reusable at once. The pool never destroys a texture, and reusing one
    /// is ordered by the queue like any other write — the three-frame wait radv needs is for a
    /// texture *dropped* in flight. Waiting here held four targets where one does (#1201).
    pub(super) fn claim(&mut self, desc: TargetDesc) -> (u32, Fresh) {
        let frame = self.frame;
        let free = self
            .slots
            .iter()
            .position(|slot| slot.alive && !slot.held && slot.desc == desc);
        if let Some(index) = free {
            let slot = &mut self.slots[index];
            slot.held = true;
            slot.last_used = frame;
            return (index as u32, Fresh::Reused);
        }
        let slot = Slot {
            desc,
            held: true,
            alive: true,
            last_used: frame,
        };
        match self.slots.iter().position(|slot| !slot.alive) {
            Some(index) => {
                self.slots[index] = slot;
                (index as u32, Fresh::Created)
            }
            None => {
                self.slots.push(slot);
                ((self.slots.len() - 1) as u32, Fresh::Created)
            }
        }
    }

    /// Closes a frame, and returns the slots nobody claimed for [`KEEP_FRAMES`]: their textures go.
    ///
    /// 🔴 Without this a resize leaked: every size a view passed through kept its target, and
    /// dragging a panel edge left hundreds of them (#1196 smoke test).
    pub(super) fn end_frame(&mut self) -> Vec<u32> {
        self.frame += 1;
        let frame = self.frame;
        let mut evicted = Vec::new();
        for (index, slot) in self.slots.iter_mut().enumerate() {
            if slot.alive && !slot.held && frame - slot.last_used > KEEP_FRAMES {
                slot.alive = false;
                evicted.push(index as u32);
            }
        }
        evicted
    }

    pub(super) fn release(&mut self, index: u32) {
        if let Some(slot) = self.slots.get_mut(index as usize) {
            slot.held = false;
        }
    }

    pub(super) fn desc(&self, index: u32) -> Option<TargetDesc> {
        self.slots
            .get(index as usize)
            .filter(|slot| slot.alive)
            .map(|slot| slot.desc)
    }

    pub(super) fn len(&self) -> usize {
        self.slots.len()
    }

    /// How many slots nothing holds.
    #[cfg(test)]
    pub(super) fn free(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| slot.alive && !slot.held)
            .count()
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
