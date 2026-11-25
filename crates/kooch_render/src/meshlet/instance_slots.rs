//! Instances that keep their row from frame to frame (#2014): a drawn mesh holds one slot and one
//! LOD-group range for as long as it draws, so one entering or leaving moves no other row.

use std::collections::HashMap;

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
}

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
                true => region.place(instance, key, count, frame),
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
            free: Holes::aligned(1),
            free_groups: Holes::aligned(1),
            group_end: 0,
        }
    }
}

impl Region {
    fn place(&mut self, mut instance: MeshInstance, key: DrawKey, groups: u32, frame: u32) {
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
    }

    /// Frees every slot not placed this frame, and drops the free tail.
    fn retire(&mut self, frame: u32, empty: u32, renumbered: bool) {
        for slot in 0..self.rows.len() {
            if self.seen[slot] == frame {
                continue;
            }
            match self.key[slot].take() {
                Some(key) => {
                    self.of_key.remove(&key);
                    self.free.give(Span {
                        start: slot as u32,
                        len: 1,
                    });
                    self.free_groups
                        .give(std::mem::take(&mut self.groups[slot]));
                }
                None if !renumbered => continue,
                None => {}
            }
            self.rows[slot] = free_row(empty);
            self.previous[slot] = free_row(empty).transform;
        }
        let len = self.free.trim(self.rows.len() as u32) as usize;
        self.key.truncate(len);
        self.rows.truncate(len);
        self.previous.truncate(len);
        self.groups.truncate(len);
        self.seen.truncate(len);
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
