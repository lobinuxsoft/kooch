//! When an old block leaves the screen and a new one takes its place. A block that left the pool
//! lingers until every block replacing it has its mesh, and a new block waits hidden while an old
//! one still overlaps it, so a level change never opens a hole or draws two levels at once.

use std::collections::HashSet;

use kooch_core::resource::Resources;
use kooch_render::meshlet::GpuMeshes;

use super::{Blocks, PARTS};
use crate::select::root_level;
use crate::{BlockKey, BlockPool, Planets};

/// Frames a block may linger when what replaces it never arrives — the pool thrashing, or a
/// generator that stopped answering — so the draws beyond the pool stay bounded.
const LINGER: u32 = 240;

impl Blocks {
    /// Retires the blocks whose replacements are in and shows the new ones nothing overlaps. Runs
    /// before this frame's meshes are drawn, so every block it shows has a propagated transform.
    pub(crate) fn swap(&mut self, resources: &mut Resources, pool: &BlockPool, planets: &Planets) {
        self.frame = self.frame.wrapping_add(1);
        if self.settled {
            return;
        }
        let Some(mut meshes) = resources.remove::<GpuMeshes>() else {
            return;
        };
        let roots: Vec<u8> = planets.shape.iter().map(root_level).collect();
        let root = |key: BlockKey| roots.get(key.planet as usize).copied().unwrap_or(0);
        // Every block holding a resident one that has no mesh yet.
        let mut awaited = HashSet::new();
        for key in pool.keys().filter(|key| !self.built(*key)) {
            for up in key.ancestors(root(key)) {
                if !awaited.insert(up) {
                    break;
                }
            }
        }
        let mut gone = Vec::new();
        let mut lingering = HashSet::new();
        for (&key, &i) in &self.of_key {
            let s = i as usize;
            if pool.slot(key).is_some() {
                self.left[s] = None;
                continue;
            }
            let left = *self.left[s].get_or_insert(self.frame);
            let merging = key
                .ancestors(root(key))
                .find(|up| pool.slot(*up).is_some())
                .is_some_and(|up| !self.built(up));
            // Only what is on screen lingers: a hidden block holds nothing up.
            let waited = self.shown[s] && (awaited.contains(&key) || merging);
            if waited && self.frame.wrapping_sub(left) <= LINGER {
                lingering.insert(key);
            } else {
                gone.push(i);
            }
        }
        for i in gone {
            self.retire(i, &mut meshes);
        }
        // Every block holding a lingering one: a new block over or under it waits for it to go.
        let mut over = HashSet::new();
        for &key in &lingering {
            for up in key.ancestors(root(key)) {
                if !over.insert(up) {
                    break;
                }
            }
        }
        let mut waiting = 0;
        let mut ready = Vec::new();
        for (&key, &i) in &self.of_key {
            // A new block waits for both its meshes: an interior shown without its seam is a hole.
            let s = i as usize;
            let staged = self.staged[s * PARTS..(s + 1) * PARTS]
                .iter()
                .any(Option::is_some);
            if self.shown[s] || !staged || !self.complete(s) {
                continue;
            }
            let covered =
                over.contains(&key) || key.ancestors(root(key)).any(|up| lingering.contains(&up));
            match covered {
                true => waiting += 1,
                false => ready.push(i),
            }
        }
        for i in ready {
            self.show(i, &mut meshes);
        }
        resources.insert(meshes);
        self.settled = awaited.is_empty() && lingering.is_empty() && waiting == 0;
    }

    /// Puts the staged meshes on screen and gives back the ones they replace.
    fn show(&mut self, i: u32, meshes: &mut GpuMeshes) {
        let s = i as usize;
        for d in s * PARTS..(s + 1) * PARTS {
            let Some(mesh) = self.staged[d].take() else {
                continue;
            };
            if let Some(old) = self.mesh[d].replace(mesh) {
                meshes.release(old);
            }
        }
        self.shown[s] = true;
    }

    /// Hides the index, gives its meshes back and frees it.
    /// A grant still on its way finds no ask and goes back too.
    fn retire(&mut self, i: u32, meshes: &mut GpuMeshes) {
        let s = i as usize;
        self.drop_meshes(s, meshes);
        if let Some(key) = self.key[s].take() {
            self.of_key.remove(&key);
        }
        self.left[s] = None;
        for d in s * PARTS..(s + 1) * PARTS {
            self.asked[d] = None;
            self.held[d] = false;
            self.triangles[d] = 0;
            self.meshlets[d] = 0;
        }
        self.free.push(i);
    }
}
