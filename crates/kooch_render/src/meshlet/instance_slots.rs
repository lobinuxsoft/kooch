//! Instances that keep their row from frame to frame (#2014): a drawn mesh holds one slot and one
//! LOD-group range for as long as it draws, so one entering or leaving moves no other row.

use std::collections::HashMap;
use std::ops::Range;

use super::DrawKey;
use super::pool::{Holes, Span};
use super::scene::{INSTANCE_EMPTY, INSTANCE_TRANSPARENT, MeshInstance};

type Matrix = [[f32; 4]; 4];

/// The rows of every drawn mesh: the opaque ones first, the transparent ones after them, as every
/// cull and the forward pass expect.
#[derive(Default)]
pub(crate) struct InstanceSlots {
    opaque: Region,
    transparent: Region,
    frame: u32,
    empty: u32,
    rows: Vec<MeshInstance>,
    previous: Vec<Matrix>,
    arrived: Vec<(MeshInstance, DrawKey, u32)>,
    /// What the GPU holds, row for row: only the rows that differ are written.
    sent: Vec<MeshInstance>,
    sent_previous: Vec<Matrix>,
    moved: Vec<u32>,
    dirty: Vec<u32>,
    runs: Vec<Range<usize>>,
}

/// Clean rows a run may span to save a write: 64 rows are 8 KiB.
const GAP: usize = 64;

impl InstanceSlots {
    /// Places this frame's drawn meshes, frees the slots of those no longer drawn, and lays out the
    /// rows. `groups(mesh_id)` is a mesh's LOD-group count; `empty` a mesh with no meshlets, which a
    /// free slot points at so it culls to nothing in every pass.
    pub fn place(
        &mut self,
        drawn: impl IntoIterator<Item = (MeshInstance, DrawKey)>,
        groups: impl Fn(u32) -> u32,
        empty: u32,
    ) {
        self.frame = self.frame.wrapping_add(1);
        let frame = self.frame;
        // Known meshes first, then the slots of those gone are freed, and only then the new ones
        // are placed: a block that leaves as another arrives hands it its slot.
        let mut arrived = std::mem::take(&mut self.arrived);
        for (instance, key) in drawn {
            let count = groups(instance.mesh_id);
            let region = self.region(instance.flags);
            match region.of_key.contains_key(&key) {
                true => {
                    region.place(instance, key, count, frame);
                }
                false => arrived.push((instance, key, count)),
            }
        }
        // A defragment renumbers the empty mesh too; the free rows follow it.
        let renumbered = self.empty != empty;
        self.empty = empty;
        for region in [&mut self.opaque, &mut self.transparent] {
            region.retire(frame, empty, renumbered);
        }
        for (instance, key, count) in arrived.drain(..) {
            self.region(instance.flags)
                .place(instance, key, count, frame);
        }
        self.arrived = arrived;
        self.lay_out();
    }

    /// Starts the kept rows' frame: those changed last frame stand still now, so their previous
    /// transform catches up. Before any [`Self::keep`].
    pub fn settle(&mut self) {
        for region in [&mut self.opaque, &mut self.transparent] {
            for slot in region.touched.drain(..) {
                let slot = slot as usize;
                if let (Some(previous), Some(row)) =
                    (region.previous.get_mut(slot), region.rows.get(slot))
                {
                    *previous = row.transform;
                }
            }
        }
    }

    /// Places a row its producer keeps (#2014): it stays until [`Self::unkeep`], not only while
    /// [`Self::place`] is handed it.
    pub fn keep(&mut self, instance: MeshInstance, key: DrawKey, groups: u32) {
        let empty = self.empty;
        let frame = self.frame;
        match instance.flags & INSTANCE_TRANSPARENT {
            0 => self.transparent.unkeep(key, empty),
            _ => self.opaque.unkeep(key, empty),
        }
        let region = self.region(instance.flags);
        let slot = region.place(instance, key, groups, frame);
        region.kept[slot] = true;
        region.touched.push(slot as u32);
    }

    /// Frees a kept row.
    pub fn unkeep(&mut self, key: DrawKey) {
        let empty = self.empty;
        self.opaque.unkeep(key, empty);
        self.transparent.unkeep(key, empty);
    }

    /// Frees every kept row, to keep them all again.
    pub fn unkeep_all(&mut self) {
        let empty = self.empty;
        for region in [&mut self.opaque, &mut self.transparent] {
            let kept: Vec<DrawKey> = (0..region.key.len())
                .filter(|&slot| region.kept[slot])
                .filter_map(|slot| region.key[slot])
                .collect();
            for key in kept {
                region.unkeep(key, empty);
            }
        }
    }

    /// Compares the rows with what the GPU holds. Call after the last write to the rows.
    pub fn diff(&mut self) {
        self.moved.clear();
        self.dirty.clear();
        for (i, row) in self.rows.iter().enumerate() {
            let moved = self
                .sent
                .get(i)
                .is_none_or(|sent| bytemuck::bytes_of(sent) != bytemuck::bytes_of(row));
            // A row standing still still owes one write: its previous transform catches up.
            let caught_up = self.sent_previous.get(i).is_some_and(|sent| {
                bytemuck::bytes_of(sent) == bytemuck::bytes_of(&self.previous[i])
            });
            if moved {
                self.moved.push(i as u32);
            }
            if moved || !caught_up {
                self.dirty.push(i as u32);
            }
        }
        let len = self.rows.len();
        self.sent.truncate(len);
        self.sent_previous.truncate(len);
        // Ascending, and every row past the old end is dirty, so a push lands at its index.
        for &i in &self.dirty {
            let i = i as usize;
            match self.sent.get_mut(i) {
                Some(sent) => *sent = self.rows[i],
                None => self.sent.push(self.rows[i]),
            }
            match self.sent_previous.get_mut(i) {
                Some(sent) => *sent = self.previous[i],
                None => self.sent_previous.push(self.previous[i]),
            }
        }
        self.runs.clear();
        for &i in &self.dirty {
            let i = i as usize;
            match self.runs.last_mut() {
                Some(run) if i <= run.end + GAP => run.end = i + 1,
                _ => self.runs.push(i..i + 1),
            }
        }
    }

    /// The GPU lost every row: a grown buffer starts empty.
    pub fn resend(&mut self) {
        self.sent.clear();
        self.sent_previous.clear();
    }

    /// Rows whose own bytes changed since the last [`Self::diff`]: what moves a shadow.
    pub fn moved(&self) -> &[u32] {
        &self.moved
    }

    /// Row ranges to write, from the last [`Self::diff`].
    pub fn runs(&self) -> &[Range<usize>] {
        &self.runs
    }

    /// Every row, opaque then transparent.
    pub fn rows(&self) -> &[MeshInstance] {
        &self.rows
    }

    /// Mutable for the per-frame flags written after placement (#452).
    pub fn rows_mut(&mut self) -> &mut [MeshInstance] {
        &mut self.rows
    }

    /// Each row's transform last frame, for the motion vectors (#481).
    pub fn previous(&self) -> &[Matrix] {
        &self.previous
    }

    /// How many rows lead before the first transparent one.
    pub fn opaque(&self) -> usize {
        self.opaque.rows.len()
    }

    /// LOD-group slots the rows span, holes included: what `group_max_err` must hold.
    pub fn groups(&self) -> u32 {
        self.opaque.group_end + self.transparent.group_end
    }

    fn region(&mut self, flags: u32) -> &mut Region {
        match flags & INSTANCE_TRANSPARENT {
            0 => &mut self.opaque,
            _ => &mut self.transparent,
        }
    }

    fn lay_out(&mut self) {
        let offset = self.opaque.group_end;
        self.rows.clear();
        self.rows.extend_from_slice(&self.opaque.rows);
        self.rows
            .extend(self.transparent.rows.iter().map(|row| MeshInstance {
                group_base: row.group_base + offset,
                ..*row
            }));
        self.previous.clear();
        self.previous.extend_from_slice(&self.opaque.previous);
        self.previous.extend_from_slice(&self.transparent.previous);
    }
}

/// One run of rows, as parallel arrays indexed by slot.
struct Region {
    of_key: HashMap<DrawKey, u32>,
    key: Vec<Option<DrawKey>>,
    rows: Vec<MeshInstance>,
    previous: Vec<Matrix>,
    groups: Vec<Span>,
    seen: Vec<u32>,
    /// Held by a producer until it lets go, whatever [`InstanceSlots::place`] is handed.
    kept: Vec<bool>,
    /// Kept slots changed this frame, whose previous transform catches up next frame.
    touched: Vec<u32>,
    free: Holes,
    free_groups: Holes,
    group_end: u32,
}

impl Default for Region {
    fn default() -> Self {
        Self {
            of_key: HashMap::new(),
            key: Vec::new(),
            rows: Vec::new(),
            previous: Vec::new(),
            groups: Vec::new(),
            seen: Vec::new(),
            kept: Vec::new(),
            touched: Vec::new(),
            free: Holes::aligned(1),
            free_groups: Holes::aligned(1),
            group_end: 0,
        }
    }
}

impl Region {
    fn place(
        &mut self,
        mut instance: MeshInstance,
        key: DrawKey,
        groups: u32,
        frame: u32,
    ) -> usize {
        let slot = match self.of_key.get(&key) {
            Some(&slot) => slot as usize,
            None => {
                let slot = self.open();
                self.of_key.insert(key, slot as u32);
                self.key[slot] = Some(key);
                // New this frame: it moved from nowhere, so no motion.
                self.rows[slot].transform = instance.transform;
                slot
            }
        };
        if self.groups[slot].len != groups {
            let old = std::mem::take(&mut self.groups[slot]);
            self.free_groups.give(old);
            self.groups[slot] = self.take_groups(groups);
        }
        instance.group_base = self.groups[slot].start;
        self.previous[slot] = self.rows[slot].transform;
        self.rows[slot] = instance;
        self.seen[slot] = frame;
        slot
    }

    fn unkeep(&mut self, key: DrawKey, empty: u32) {
        let Some(slot) = self.of_key.remove(&key) else {
            return;
        };
        self.free_slot(slot as usize, empty);
    }

    fn free_slot(&mut self, slot: usize, empty: u32) {
        self.key[slot] = None;
        self.kept[slot] = false;
        self.free.give(Span {
            start: slot as u32,
            len: 1,
        });
        self.free_groups
            .give(std::mem::take(&mut self.groups[slot]));
        self.rows[slot] = free_row(empty);
        self.previous[slot] = free_row(empty).transform;
    }

    /// Frees every slot not placed this frame, and drops the free tail.
    fn retire(&mut self, frame: u32, empty: u32, renumbered: bool) {
        for slot in 0..self.rows.len() {
            if self.seen[slot] == frame || self.kept[slot] {
                continue;
            }
            match self.key[slot] {
                Some(key) => {
                    self.of_key.remove(&key);
                    self.free_slot(slot, empty);
                }
                None if renumbered => {
                    self.rows[slot] = free_row(empty);
                    self.previous[slot] = free_row(empty).transform;
                }
                None => {}
            }
        }
        let len = self.free.trim(self.rows.len() as u32) as usize;
        self.key.truncate(len);
        self.rows.truncate(len);
        self.previous.truncate(len);
        self.groups.truncate(len);
        self.seen.truncate(len);
        self.kept.truncate(len);
        self.group_end = self.free_groups.trim(self.group_end);
    }

    fn open(&mut self) -> usize {
        if let Some(span) = self.free.take(1) {
            return span.start as usize;
        }
        self.key.push(None);
        self.rows.push(MeshInstance::default());
        self.previous.push(MeshInstance::default().transform);
        self.groups.push(Span::default());
        self.seen.push(0);
        self.kept.push(false);
        self.rows.len() - 1
    }

    fn take_groups(&mut self, len: u32) -> Span {
        if let Some(span) = self.free_groups.take(len) {
            return span;
        }
        let span = Span {
            start: self.group_end,
            len,
        };
        self.group_end += len;
        span
    }
}

/// A row no mesh holds: no layer, so every cull skips it, and a mesh with no meshlets, so a pass
/// that reads it without culling draws nothing either.
fn free_row(empty: u32) -> MeshInstance {
    MeshInstance {
        flags: INSTANCE_EMPTY,
        layers: 0,
        ..MeshInstance::new(glam::Mat4::IDENTITY, empty, 0)
    }
}

#[cfg(test)]
mod tests;
