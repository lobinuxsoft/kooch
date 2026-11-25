//! #1999 debug: whether a seam on screen was counted against the neighbours on screen. Only around
//! what changed this frame, and only against neighbours with geometry: an empty one opens no hole.

use std::collections::HashSet;

use glam::IVec3;

use super::{Blocks, PARTS};
use crate::mesh::Part;
use crate::seam::{FINER, OWN, SAME, direction, levels};
use crate::{BlockKey, BlockPool};

#[derive(Debug, PartialEq)]
enum Side {
    Same,
    Finer,
    Coarser,
    None,
}

impl Blocks {
    pub(super) fn audit(&mut self, pool: &BlockPool) {
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
        let left = std::mem::take(&mut self.left_solid);
        // Per retired block, a resident block anywhere under it: drawn or not, the first found.
        let gone: HashSet<BlockKey> = left.iter().map(|(k, _)| *k).collect();
        let mut under: std::collections::HashMap<BlockKey, BlockKey> = Default::default();
        if !gone.is_empty() {
            for k in pool.keys() {
                let ups = std::iter::successors(Some(k.parent()), |u| Some(u.parent())).take(40);
                for up in ups.filter(|u| gone.contains(u)) {
                    let hidden = !self.drawn(k);
                    let slot = under.entry(up).or_insert(k);
                    if hidden && self.drawn(*slot) {
                        *slot = k;
                    }
                }
            }
        }
        for (key, interior) in left {
            let Some(hole) = self.hole(key, pool, under.get(&key).copied()) else {
                continue;
            };
            self.bare += 1;
            if self.bare_example.is_none() {
                self.bare_example = Some(format!("{key:?} left, {hole}, interior={interior}"));
            }
            if interior && self.bare_key.is_none() {
                self.bare_key = Some(key);
            }
        }
        self.changed = changed;
        self.changed.clear();
    }

    fn shown(&self, key: BlockKey) -> bool {
        self.of_key
            .get(&key)
            .is_some_and(|&i| self.shown[i as usize])
    }

    /// Whether `key` or an ancestor is on screen.
    fn drawn(&self, key: BlockKey) -> bool {
        std::iter::successors(Some(key), |k| Some(k.parent()))
            .take(32)
            .any(|k| self.shown(k))
    }

    /// A resident block in or around `key`'s space that nothing on screen draws: what replaces
    /// `key` has not appeared yet. Also when the pool turned away a block meant to replace it.
    /// `under` is a resident block anywhere beneath it, a hidden one if there is.
    fn hole(&self, key: BlockKey, pool: &BlockPool, under: Option<BlockKey>) -> Option<String> {
        if self.drawn(key) {
            return None;
        }
        let above = std::iter::successors(Some(key.parent()), |k| Some(k.parent())).take(32);
        for k in under.into_iter().chain(above) {
            if pool.slot(k).is_some() && !self.drawn(k) {
                let built = self.built(k);
                return Some(format!("level {} resident, hidden, built={built}", k.level));
            }
        }
        under
            .is_none()
            .then(|| "nothing resident under or over it".to_string())
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
