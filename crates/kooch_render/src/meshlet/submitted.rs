//! Renderers with no entity of their own: a producer that draws many meshes for one entity — a
//! planet's blocks (#1991) — hands them here instead of spawning an entity per mesh.
//!
//! 🔴 Kept, not rebuilt (#2014): a producer sets and removes rows as they change, and the renderer
//! reads only the changes, so a row that did not change costs nothing on its frame.

use std::collections::HashMap;

use glam::Mat4;
use kooch_core::Guid;
use kooch_core::resource::Resources;
use kooch_ecs::entity::Entity;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_ecs::mesh_renderer::MeshRenderer;
use kooch_ecs::query::Query;

/// What a drawn mesh is from frame to frame: its entity, and which of the entity's meshes. An
/// entity's own renderer is part 0; a producer names its parts, which must stay put while they draw,
/// or last frame's matrix is looked up for the wrong mesh (#481).
pub type DrawKey = (Entity, u32);

/// The entity-less renderers, as parallel arrays, with matrices relative to the world origin of the
/// frame that set them.
#[derive(Default)]
pub struct Submitted {
    of_key: HashMap<DrawKey, u32>,
    key: Vec<DrawKey>,
    matrix: Vec<Mat4>,
    renderer: Vec<MeshRenderer>,
    /// Every key changed since the previous frame began, with the change count it took.
    log: Vec<(u64, DrawKey)>,
    changes: u64,
    /// `changes` when the previous frame and this one began.
    began: [u64; 2],
    /// How many rows draw with each material, so what is drawn is known without a walk.
    materials: HashMap<Guid, u32>,
}

impl Submitted {
    /// Starts a frame: the log keeps this frame's changes and the previous one's.
    pub fn begin(&mut self) {
        self.began = [self.began[1], self.changes];
        let floor = self.began[0];
        self.log.retain(|&(at, _)| at > floor);
    }

    /// Sets the row drawn under `key`, adding it if new.
    pub fn set(&mut self, key: DrawKey, matrix: Mat4, renderer: MeshRenderer) {
        self.count(renderer.material, 1);
        match self.of_key.get(&key) {
            Some(&i) => {
                let i = i as usize;
                self.matrix[i] = matrix;
                let old = std::mem::replace(&mut self.renderer[i], renderer);
                self.count(old.material, -1);
            }
            None => {
                self.of_key.insert(key, self.key.len() as u32);
                self.key.push(key);
                self.matrix.push(matrix);
                self.renderer.push(renderer);
            }
        }
        self.logged(key);
    }

    /// Stops drawing `key`.
    pub fn remove(&mut self, key: DrawKey) {
        let Some(i) = self.of_key.remove(&key) else {
            return;
        };
        let i = i as usize;
        self.key.swap_remove(i);
        self.matrix.swap_remove(i);
        let old = self.renderer.swap_remove(i);
        self.count(old.material, -1);
        if let Some(&moved) = self.key.get(i) {
            self.of_key.insert(moved, i as u32);
        }
        self.logged(key);
    }

    /// The row drawn under `key`.
    pub fn get(&self, key: DrawKey) -> Option<(&Mat4, &MeshRenderer)> {
        let i = *self.of_key.get(&key)? as usize;
        Some((self.matrix.get(i)?, self.renderer.get(i)?))
    }

    /// Every key drawn.
    pub fn keys(&self) -> &[DrawKey] {
        &self.key
    }

    /// How many changes have been made: a reader's place in the log.
    pub fn changes(&self) -> u64 {
        self.changes
    }

    /// The keys changed after `seen`, or `None` once the log no longer reaches back to it: the
    /// reader then reads every row again.
    pub fn since(&self, seen: u64) -> Option<impl Iterator<Item = DrawKey> + '_> {
        if seen < self.began[0] {
            return None;
        }
        let from = self.log.partition_point(|&(at, _)| at <= seen);
        Some(self.log[from..].iter().map(|&(_, key)| key))
    }

    /// Every material a row draws with.
    pub fn materials(&self) -> impl Iterator<Item = Guid> + '_ {
        self.materials.keys().copied()
    }

    fn logged(&mut self, key: DrawKey) {
        self.changes += 1;
        self.log.push((self.changes, key));
    }

    fn count(&mut self, material: Option<Guid>, by: i32) {
        let Some(material) = material else {
            return;
        };
        let rows = self.materials.entry(material).or_default();
        *rows = rows.saturating_add_signed(by);
        if *rows == 0 {
            self.materials.remove(&material);
        }
    }
}

/// Every entity's renderer. 🔴 Every walk that asks what is drawn goes through here and reads
/// [`Submitted`] beside it, or a submitted mesh or its material is evicted while on screen.
pub fn for_each_drawn(resources: &Resources, mut each: impl FnMut(DrawKey, &MeshRenderer, &Mat4)) {
    Query::<(&MeshRenderer, &GlobalTransform)>::new(resources).for_each_entity(
        |entity, (renderer, transform)| each((entity, 0), renderer, &transform.matrix),
    );
}

#[cfg(test)]
mod tests;
