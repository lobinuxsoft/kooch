//! #1999 debug: whether a seam on screen was counted against the neighbours on screen. Only around
//! what changed this frame, and only against neighbours with geometry: an empty one opens no hole.

use std::collections::HashSet;

use glam::IVec3;

use super::{Blocks, PARTS};
use crate::BlockKey;
use crate::mesh::Part;
use crate::seam::{FINER, OWN, SAME, direction, levels};

#[derive(Debug, PartialEq)]
enum Side {
    Same,
    Finer,
    Coarser,
    None,
}

impl Blocks {
    pub(super) fn audit(&mut self) {
        let changed = std::mem::take(&mut self.changed);
        let mut around = HashSet::new();
        for &key in &changed {
            around.insert(key);
            for dir in 0..26 {
                let across = BlockKey {
                    cell: key.cell + direction(dir),
                    ..key
                };
                around.insert(across);
                around.insert(across.parent());
                around.extend(children(across));
            }
        }
        for key in around {
            self.check(key);
        }
        self.changed = changed;
        self.changed.clear();
    }

    /// On screen with geometry: the only kind a seam can tear against.
    fn solid(&self, key: BlockKey) -> bool {
        self.of_key.get(&key).is_some_and(|&i| {
            let s = i as usize;
            self.shown[s] && (s * PARTS..(s + 1) * PARTS).any(|d| self.mesh[d].is_some())
        })
    }

    fn check(&mut self, key: BlockKey) {
        let Some(&i) = self.of_key.get(&key) else {
            return;
        };
        let s = i as usize;
        let d = s * PARTS + Part::Seam as usize;
        if !self.shown[s] || self.mesh[d].is_none() {
            return;
        }
        let near = self.near[d];
        for (dir, &side) in near.sides.iter().enumerate() {
            let across = BlockKey {
                cell: key.cell + direction(dir),
                ..key
            };
            let seen = if self.solid(across) {
                Side::Same
            } else if children(across).into_iter().any(|c| self.solid(c)) {
                Side::Finer
            } else if self.solid(across.parent()) {
                Side::Coarser
            } else {
                Side::None
            };
            let fits = seen == Side::None
                || match side {
                    FINER => seen == Side::Finer,
                    OWN => seen == Side::Same,
                    _ if levels(side) > 0 => seen == Side::Coarser,
                    _ if side & SAME != 0 => seen == Side::Same,
                    _ => true,
                };
            if fits {
                continue;
            }
            self.torn += 1;
            if self.torn_example.is_none() {
                self.torn_example = Some(format!(
                    "{key:?} dir {:?} counted {side:#x} sees {seen:?} staged={} pending={} asked={}",
                    direction(dir),
                    self.staged[d].is_some(),
                    self.pending[d],
                    self.asked[d].is_some(),
                ));
            }
            return;
        }
    }
}

fn children(key: BlockKey) -> [BlockKey; 8] {
    std::array::from_fn(|i| {
        let i = i as i32;
        BlockKey {
            planet: key.planet,
            level: key.level.wrapping_sub(1),
            cell: key.cell * 2 + IVec3::new(i & 1, (i >> 1) & 1, (i >> 2) & 1),
        }
    })
}
