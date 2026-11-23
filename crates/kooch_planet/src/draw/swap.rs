//! When meshes go on screen, as one transaction per frame. A block that left the pool lingers until
//! every block replacing it is shown; a staged mesh is shown only when nothing lingering overlaps it
//! and it agrees with every neighbour it will be drawn beside. Holding one back can hold others, so
//! the set is shrunk until it stands, and what is left goes on screen in the same frame.

use std::collections::HashSet;

use kooch_core::resource::Resources;
use kooch_ecs::MeshRenderer;
use kooch_ecs::query::Query;
use kooch_render::meshlet::GpuMeshes;

use super::Blocks;
use super::agree::{Shows, agrees};
use crate::select::root_level;
use crate::{BlockKey, BlockPool, Planets};

/// Frames a block may linger, or a mesh wait for its neighbours, when what it waits for never
/// arrives — the pool thrashing, or a generator that stopped answering — so the screen still moves.
const LINGER: u32 = 240;

/// Shrinking passes before the frame gives up and shows nothing new.
const PASSES: u32 = 8;

/// The blocks leaving the screen and those staying on it a while longer.
#[derive(Default)]
struct Leaving {
    gone: Vec<u32>,
    lingering: HashSet<BlockKey>,
    /// Some resident block is not on screen yet.
    awaited: bool,
}

impl Blocks {
    /// Retires the blocks whose replacements are in and shows the staged meshes that stand
    /// together. Runs before this frame's meshes are drawn, so every block it shows has a
    /// propagated transform.
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
        for (&key, &i) in &self.of_key {
            let left = &mut self.left[i as usize];
            match pool.slot(key) {
                Some(_) => *left = None,
                None => _ = left.get_or_insert(self.frame),
            }
        }
        let mut take: Vec<bool> = self.staged.iter().map(Option::is_some).collect();
        let mut leaving = None;
        for _ in 0..PASSES {
            let plan = self.leaving(pool, &root, &take);
            if !self.hold_back(pool, &root, &plan.lingering, &mut take) {
                leaving = Some(plan);
                break;
            }
        }
        // It did not stand: nothing new this frame rather than a seam left open.
        let leaving = leaving.unwrap_or_else(|| {
            take.fill(false);
            self.leaving(pool, &root, &take)
        });
        for &i in &leaving.gone {
            self.retire(resources, i, &mut meshes);
        }
        for (i, _) in take.iter().enumerate().filter(|(_, take)| **take) {
            self.show(resources, i as u32, &mut meshes);
        }
        resources.insert(meshes);
        let waiting = self.staged.iter().any(Option::is_some);
        self.settled = !leaving.awaited && leaving.lingering.is_empty() && !waiting;
    }

    /// Drops from `take` every staged mesh a lingering block overlaps or a neighbour disagrees
    /// with; whether it dropped any.
    fn hold_back(
        &self,
        pool: &BlockPool,
        root: &impl Fn(BlockKey) -> u8,
        lingering: &HashSet<BlockKey>,
        take: &mut [bool],
    ) -> bool {
        let mut over = HashSet::new();
        for &key in lingering {
            for up in key.ancestors(root(key)) {
                if !over.insert(up) {
                    break;
                }
            }
        }
        let mut held = Vec::new();
        for (s, _) in take.iter().enumerate().filter(|(_, take)| **take) {
            let Some(key) = self.key[s].filter(|key| pool.slot(*key).is_some()) else {
                held.push(s);
                continue;
            };
            let covered =
                over.contains(&key) || key.ancestors(root(key)).any(|up| lingering.contains(&up));
            let late = self.frame.wrapping_sub(self.staged_at[s]) > LINGER;
            let shows = |key| self.shows(key, take);
            if covered || !(late || agrees(pool, root(key), key, &self.staged_near[s], shows)) {
                held.push(s);
            }
        }
        for &s in &held {
            take[s] = false;
        }
        !held.is_empty()
    }

    /// Which blocks out of the pool leave the screen now, given the staged meshes in `take`.
    fn leaving(&self, pool: &BlockPool, root: &impl Fn(BlockKey) -> u8, take: &[bool]) -> Leaving {
        let ready = |key| !matches!(self.shows(key, take), Shows::Missing);
        // Every block holding a resident one that is not on screen yet.
        let mut awaited = HashSet::new();
        for key in pool.keys().filter(|key| !ready(*key)) {
            for up in key.ancestors(root(key)) {
                if !awaited.insert(up) {
                    break;
                }
            }
        }
        let mut plan = Leaving {
            awaited: !awaited.is_empty(),
            ..Leaving::default()
        };
        for (&key, &i) in &self.of_key {
            let s = i as usize;
            if pool.slot(key).is_some() {
                continue;
            }
            let merging = key
                .ancestors(root(key))
                .find(|up| pool.slot(*up).is_some())
                .is_some_and(|up| !ready(up));
            // Only what is on screen lingers: a hidden block holds nothing up.
            let waited = self.shown[s] && (awaited.contains(&key) || merging);
            let left = self.left[s].unwrap_or(self.frame);
            if waited && self.frame.wrapping_sub(left) <= LINGER {
                plan.lingering.insert(key);
            } else {
                plan.gone.push(i);
            }
        }
        plan
    }

    /// What `key` will have on screen if the staged meshes in `take` go on.
    fn shows(&self, key: BlockKey, take: &[bool]) -> Shows {
        let Some(&i) = self.of_key.get(&key) else {
            return Shows::Missing;
        };
        let s = i as usize;
        if take.get(s).copied().unwrap_or(false) {
            return Shows::Mesh(self.staged_near[s]);
        }
        if self.shown[s] {
            return Shows::Mesh(self.near[s]);
        }
        let empty = self.built[s] && self.mesh[s].is_none() && self.staged[s].is_none();
        match empty {
            true => Shows::Empty,
            false => Shows::Missing,
        }
    }

    /// Puts the staged mesh on screen and gives back the one it replaces.
    fn show(&mut self, resources: &mut Resources, i: u32, meshes: &mut GpuMeshes) {
        let s = i as usize;
        let Some(mesh) = self.staged[s].take() else {
            return;
        };
        if let Some(old) = self.mesh[s].replace(mesh) {
            meshes.release(old);
        }
        self.near[s] = self.staged_near[s];
        self.shown[s] = true;
        if let Some(entity) = self.entity[s]
            && let Some(renderer) = Query::<&mut MeshRenderer>::new(resources).get(entity)
        {
            renderer.mesh = Some(mesh);
            renderer.visible = true;
        }
    }

    /// Hides the index, gives its meshes back and frees it; the entity is kept for the next block.
    /// A grant still on its way finds no ask and goes back too.
    fn retire(&mut self, resources: &mut Resources, i: u32, meshes: &mut GpuMeshes) {
        let s = i as usize;
        self.drop_meshes(resources, s, meshes);
        if let Some(key) = self.key[s].take() {
            self.of_key.remove(&key);
        }
        self.asked[s] = None;
        self.built[s] = false;
        self.left[s] = None;
        self.triangles[s] = 0;
        self.meshlets[s] = 0;
        self.free.push(i);
    }
}
