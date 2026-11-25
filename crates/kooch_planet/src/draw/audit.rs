//! #1999 debug: whether a seam on screen was counted against the neighbours on screen. A seam that
//! fits others opens a hole, and whether its fix is staged tells a held change from a lost one.

use std::collections::HashSet;

use super::{Blocks, PARTS};
use crate::BlockKey;
use crate::mesh::Part;
use crate::seam::{FINER, OWN, SAME, direction, levels};

#[derive(Debug, PartialEq)]
enum Side {
    Same,
    Finer,
    Coarser(u32),
    None,
}

impl Blocks {
    pub(super) fn audit(&mut self, roots: &[u8]) {
        let shown: HashSet<BlockKey> = (0..self.key.len())
            .filter(|&s| self.shown[s])
            .filter_map(|s| self.key[s])
            .collect();
        let mut holding = HashSet::new();
        for &key in &shown {
            let root = roots.get(key.planet as usize).copied().unwrap_or(0);
            holding.extend(key.ancestors(root));
        }
        for s in (0..self.key.len()).filter(|&s| self.shown[s]) {
            let Some(key) = self.key[s] else { continue };
            let d = s * PARTS + Part::Seam as usize;
            if self.mesh[d].is_none() {
                continue;
            }
            let root = roots.get(key.planet as usize).copied().unwrap_or(0);
            let near = self.near[d];
            for (dir, &side) in near.sides.iter().enumerate() {
                let across = BlockKey {
                    cell: key.cell + direction(dir),
                    ..key
                };
                let seen = if shown.contains(&across) {
                    Side::Same
                } else if holding.contains(&across) {
                    Side::Finer
                } else {
                    across
                        .ancestors(root)
                        .find(|up| shown.contains(up))
                        .map_or(Side::None, |up| {
                            Side::Coarser(u32::from(up.level - key.level))
                        })
                };
                let fits = match side {
                    FINER => seen == Side::Finer,
                    OWN => matches!(seen, Side::Same | Side::None),
                    _ if levels(side) > 0 => seen == Side::Coarser(levels(side)),
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
                break;
            }
        }
    }
}
