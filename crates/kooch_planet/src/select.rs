//! Budgeted octree refinement: the leaves of every planet's octree, split highest error first
//! until the pool is full, and 2:1 balanced: no two touching leaves are more than one level apart,
//! so a seam between blocks meets at most one level change (#393).

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashSet};

use glam::{DVec3, IVec3};

use super::{BLOCK_SAMPLES, BlockKey};
use crate::sight::Cone;

/// How much a block split last frame overstates its error: it merges back only once it is well under
/// the target, so a block at the threshold does not split and merge on alternate frames.
const KEEP_SPLIT: f64 = 1.5;

/// What a planet is made of, in metres.
#[derive(Clone, Copy, Debug)]
pub struct Shape {
    /// Sea-level radius.
    pub radius: f64,
    /// Highest relief above the radius.
    pub relief: f64,
    /// Deepest cave below the radius; the core under it is solid and needs no blocks.
    pub depth: f64,
    /// Edge of a voxel at the finest level.
    pub voxel: f64,
    /// Reach of the finest level from the camera; each coarser level reaches twice as far.
    pub lod_distance: f64,
}

/// Blocks a planet gets before the others compete, and the most it may hold.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub floor: u32,
    pub ceiling: u32,
}

/// Every planet in view, as parallel arrays indexed by planet.
#[derive(Clone, Debug, Default)]
pub struct Planets {
    pub shape: Vec<Shape>,
    /// Every viewer in the planet's local space, its origin and rotation removed: `views` in a
    /// row per planet. A block is refined for the viewer closest to it.
    pub viewer: Vec<DVec3>,
    /// Each viewer's view cone, parallel to `viewer`.
    pub cone: Vec<Cone>,
    pub views: usize,
    pub limits: Vec<Limits>,
}

impl Planets {
    /// Adds a planet and returns its index, which is `BlockKey::planet`.
    /// Every planet takes as many viewers as the first one did; a planet with none is out of view.
    pub fn push(&mut self, shape: Shape, viewers: &[DVec3], limits: Limits) -> u32 {
        self.push_seen(shape, viewers, &[], limits)
    }

    /// [`Planets::push`] with each viewer's view cone; a viewer without one sees everything.
    pub fn push_seen(
        &mut self,
        shape: Shape,
        viewers: &[DVec3],
        cones: &[Cone],
        limits: Limits,
    ) -> u32 {
        if self.shape.is_empty() {
            self.viewer.clear();
            self.cone.clear();
            self.views = viewers.len();
        }
        self.cone
            .extend((0..self.views).map(|v| cones.get(v).copied().unwrap_or(Cone::ALL)));
        let nowhere = DVec3::splat(f64::INFINITY);
        self.viewer.extend((0..self.views).map(|v| {
            viewers
                .get(v)
                .or(viewers.last())
                .copied()
                .unwrap_or(nowhere)
        }));
        self.shape.push(shape);
        self.limits.push(limits);
        (self.shape.len() - 1) as u32
    }

    /// Empties it for the next frame, keeping its allocations.
    pub fn clear(&mut self) {
        self.shape.clear();
        self.viewer.clear();
        self.cone.clear();
        self.views = 0;
        self.limits.clear();
    }

    /// `planet`'s viewers, in its local space.
    pub fn viewers(&self, planet: usize) -> &[DVec3] {
        self.viewer
            .get(planet * self.views..(planet + 1) * self.views)
            .unwrap_or_default()
    }

    /// `planet`'s view cones, parallel to [`Planets::viewers`].
    pub fn cones(&self, planet: usize) -> &[Cone] {
        self.cone
            .get(planet * self.views..(planet + 1) * self.views)
            .unwrap_or_default()
    }

    pub fn len(&self) -> usize {
        self.shape.len()
    }

    pub fn is_empty(&self) -> bool {
        self.shape.is_empty()
    }
}

/// The camera the error is measured against.
#[derive(Clone, Copy, Debug)]
pub struct View {
    /// Pixels per metre at one metre: `viewport_height / (2 · tan(fov_y / 2))`.
    pub proj_scale: f64,
    /// A planet drawn smaller than this keeps its root blocks and stops refining; its impostor
    /// (#342) will draw it instead.
    pub impostor_px: f64,
}

impl View {
    /// A view of `viewport_height` pixels and `fov_y` radians.
    pub fn new(viewport_height: f64, fov_y: f64) -> Self {
        Self {
            proj_scale: viewport_height / (2.0 * (fov_y * 0.5).tan()),
            impostor_px: 32.0,
        }
    }
}

/// Writes into `out` the blocks to keep resident — at most `capacity` — and returns the worst
/// error left, as in `Refine::error`, over blocks the budget kept from splitting.
///
/// `surface(key, min, max)` is the generator's answer to "can this block hold surface?": false
/// for a block it proves uniform (all rock or all air), which then takes no slot. Zylann answers it
/// with interval range analysis of the graph. `split(key)` says whether the block was split last
/// frame, for the hysteresis.
pub fn select(
    planets: &Planets,
    view: &View,
    capacity: u32,
    surface: impl FnMut(BlockKey, DVec3, DVec3) -> bool,
    split: impl Fn(BlockKey) -> bool,
    out: &mut Vec<BlockKey>,
) -> f64 {
    out.clear();
    let mut refine = Refine {
        planets,
        view,
        capacity,
        surface,
        split,
        counts: vec![0; planets.len()],
        total: 0,
        leaves: HashSet::new(),
    };
    // Floors first, planet by planet, so a near planet cannot starve a visible far one.
    let mut global = BinaryHeap::new();
    let mut held = Vec::new();
    for planet in 0..planets.len() {
        let mut heap = BinaryHeap::new();
        refine.seed(planet, &mut heap);
        let limits = planets.limits[planet];
        refine.split(&mut heap, &mut held, |_| limits.floor.min(limits.ceiling));
        global.extend(held.drain(..));
    }
    refine.split(&mut global, &mut held, |p| planets.limits[p].ceiling);
    // A held leaf a neighbour's split forced apart since is no leaf any more.
    let worst = held
        .iter()
        .filter(|node| refine.leaves.contains(&node.key))
        .fold(0.0, |worst: f64, node| worst.max(node.error));
    out.extend(refine.leaves.drain());
    worst
}

/// A leaf that may still split, ordered by its error.
#[derive(Clone, Copy)]
struct Node {
    error: f64,
    key: BlockKey,
}

impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Node {}

impl PartialOrd for Node {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Node {
    fn cmp(&self, other: &Self) -> Ordering {
        self.error.total_cmp(&other.error)
    }
}

struct Refine<'a, S, K> {
    planets: &'a Planets,
    view: &'a View,
    capacity: u32,
    surface: S,
    split: K,
    counts: Vec<u32>,
    total: u32,
    /// Every leaf so far: in the heap, finished or held. CPU-only bookkeeping.
    leaves: HashSet<BlockKey>,
}

impl<S: FnMut(BlockKey, DVec3, DVec3) -> bool, K: Fn(BlockKey) -> bool> Refine<'_, S, K> {
    /// The root blocks of a planet in view: eight around its centre, those that touch the shell.
    fn seed(&mut self, planet: usize, heap: &mut BinaryHeap<Node>) {
        let shape = self.planets.shape[planet];
        let outer = shape.radius + shape.relief;
        let distance = self
            .planets
            .viewers(planet)
            .iter()
            .map(|viewer| viewer.length())
            .fold(f64::INFINITY, f64::min);
        if !distance.is_finite() {
            return;
        }
        let small = distance > outer
            && 2.0 * outer / distance * self.view.proj_scale < self.view.impostor_px;
        let level = root_level(&shape);
        for i in 0..8 {
            let cell = IVec3::new(i & 1, (i >> 1) & 1, (i >> 2) & 1) - IVec3::ONE;
            let key = BlockKey {
                planet: planet as u32,
                level,
                cell,
            };
            if self.total < self.capacity && self.touches(&shape, key) {
                self.total += 1;
                self.counts[planet] += 1;
                self.leaves.insert(key);
                // Until its impostor exists, a distant planet keeps its root blocks, not nothing.
                if !small {
                    self.place(key, heap);
                }
            }
        }
    }

    /// Splits the worst leaf while the pool and `limit` allow; a leaf they stop goes to `held`.
    fn split(
        &mut self,
        heap: &mut BinaryHeap<Node>,
        held: &mut Vec<Node>,
        limit: impl Fn(usize) -> u32,
    ) {
        while let Some(node) = heap.pop() {
            // Split already, to balance a finer neighbour.
            if !self.leaves.contains(&node.key) {
                continue;
            }
            let planet = node.key.planet as usize;
            if !self.balanced_split(node.key, limit(planet), heap) {
                held.push(node);
            }
        }
    }

    /// Splits `key` once every coarser leaf touching it is split down to its level, so its children
    /// meet nothing two levels up. Each split keeps the octree balanced, so one the budget refuses
    /// halfway leaves it balanced too. False when the budget refused.
    fn balanced_split(&mut self, key: BlockKey, limit: u32, heap: &mut BinaryHeap<Node>) -> bool {
        // Balanced already: a leaf touching `key` is at most one level up, a child of a parent's
        // neighbour. The parent itself is never a leaf.
        let parent = key.parent();
        for d in 0..27 {
            let offset = IVec3::new(d % 3, (d / 3) % 3, d / 9) - IVec3::ONE;
            let across = BlockKey {
                cell: parent.cell + offset,
                ..parent
            };
            if offset == IVec3::ZERO || !self.touching(key, across) {
                continue;
            }
            // Recursion climbs a level each step, so it is bounded by the root's.
            if self.leaves.contains(&across) && !self.balanced_split(across, limit, heap) {
                return false;
            }
        }
        self.split_one(key, limit, heap)
    }

    /// Whether `coarse`, one level up, touches `key` by a face, an edge or a corner.
    fn touching(&self, key: BlockKey, coarse: BlockKey) -> bool {
        let lo = coarse.cell * 2;
        let hi = lo + IVec3::splat(2);
        let near = |a: usize| key.cell[a] >= lo[a] - 1 && key.cell[a] <= hi[a];
        near(0) && near(1) && near(2)
    }

    /// Replaces leaf `key` by its children that can hold surface, if the pool and `limit` allow.
    fn split_one(&mut self, key: BlockKey, limit: u32, heap: &mut BinaryHeap<Node>) -> bool {
        let planet = key.planet as usize;
        let shape = self.planets.shape[planet];
        let (children, count) = self.children(&shape, key);
        let grow = count.saturating_sub(1) as u32;
        if count > 0 && (self.total + grow > self.capacity || self.counts[planet] + grow > limit) {
            return false;
        }
        self.leaves.remove(&key);
        // The parent's range was looser than its children's: the block is uniform after all.
        if count == 0 {
            self.total -= 1;
            self.counts[planet] -= 1;
            return true;
        }
        self.total += grow;
        self.counts[planet] += grow;
        for &child in &children[..count] {
            self.leaves.insert(child);
            self.place(child, heap);
        }
        true
    }

    /// The children of a block that can hold surface.
    fn children(&mut self, shape: &Shape, key: BlockKey) -> ([BlockKey; 8], usize) {
        let mut out = [key; 8];
        let mut count = 0;
        for i in 0..8 {
            let offset = IVec3::new(i & 1, (i >> 1) & 1, (i >> 2) & 1);
            let child = BlockKey {
                planet: key.planet,
                level: key.level - 1,
                cell: key.cell * 2 + offset,
            };
            if self.touches(shape, child) {
                out[count] = child;
                count += 1;
            }
        }
        (out, count)
    }

    /// The planet-wide band first, as it costs nothing; then the generator.
    fn touches(&mut self, shape: &Shape, key: BlockKey) -> bool {
        let (min, max) = bounds(shape, key);
        touches_shell(shape, min, max) && (self.surface)(key, min, max)
    }

    /// Queues a leaf that can still split; one that cannot stays a finished leaf.
    fn place(&mut self, key: BlockKey, heap: &mut BinaryHeap<Node>) {
        let kept = (self.split)(key);
        let error = match kept {
            true => self.error(key) * KEEP_SPLIT,
            false => self.error(key),
        };
        // 🔴 Never by where the camera looks: turning it would re-level every block it sweeps (#1999).
        if key.level > 0 && error > 1.0 && !self.hidden(key, kept) {
            heap.push(Node { error, key });
        }
    }

    /// Whether the solid core hides the whole block from every viewer: its bounding sphere, shrunk
    /// into the core, is behind the core's horizon. Such a block stays coarse and holds its slot.
    /// A block split last frame must hide a sphere [`KEEP_SPLIT`] times larger to merge, so a
    /// viewer jittering on the horizon does not split and merge it on alternate frames.
    fn hidden(&self, key: BlockKey, kept: bool) -> bool {
        let planet = key.planet as usize;
        let shape = self.planets.shape[planet];
        let (min, max) = bounds(&shape, key);
        let center = (min + max) * 0.5;
        let grow = if kept { KEEP_SPLIT } else { 1.0 };
        let reach = shape.radius - shape.depth - (max - min).length() * 0.5 * grow;
        if !(reach > 0.0) {
            return false;
        }
        let viewers = self.planets.viewers(planet);
        !viewers.is_empty() && viewers.iter().all(|&v| behind_core(v, center, reach))
    }

    /// How many times too close for its level `key` sits, from the closest point of the block.
    fn error(&self, key: BlockKey) -> f64 {
        let planet = key.planet as usize;
        let shape = self.planets.shape[planet];
        let (min, max) = bounds(&shape, key);
        let distance = self
            .planets
            .viewers(planet)
            .iter()
            .map(|viewer| (viewer.clamp(min, max) - *viewer).length())
            .fold(f64::INFINITY, f64::min);
        // Over 1 while the block sits inside its level's ring: level L reaches lod_distance · 2^L,
        // so a level-L block belongs past the reach of level L − 1.
        shape.lod_distance * (1u64 << key.level) as f64 * 0.5 / distance
    }
}

/// Whether the segment from `viewer` to `point` passes within `reach` of the centre between its
/// ends: then any point within `core − reach` of `point` is cut off by a core of radius `core`.
fn behind_core(viewer: DVec3, point: DVec3, reach: f64) -> bool {
    let along = point - viewer;
    let length = along.length_squared();
    if !(length > 0.0) {
        return false;
    }
    let t = -viewer.dot(along) / length;
    t > 0.0 && t < 1.0 && (viewer + along * t).length() < reach
}

/// The coarsest level: the first whose block reaches past the relief from the centre.
pub(crate) fn root_level(shape: &Shape) -> u8 {
    let outer = shape.radius + shape.relief;
    let mut level = 0;
    while level < 62 && block_side(shape, level) < outer {
        level += 1;
    }
    level
}

/// Whether a block holds any of the band between the deepest cave and the highest relief; a block
/// wholly in the core or wholly in the sky is uniform and needs no slot.
fn touches_shell(shape: &Shape, min: DVec3, max: DVec3) -> bool {
    let nearest = DVec3::ZERO.clamp(min, max).length();
    let farthest = min.abs().max(max.abs()).length();
    nearest <= shape.radius + shape.relief && farthest >= shape.radius - shape.depth
}

pub(super) fn bounds(shape: &Shape, key: BlockKey) -> (DVec3, DVec3) {
    let side = block_side(shape, key.level);
    let min = key.cell.as_dvec3() * side;
    (min, min + DVec3::splat(side))
}

fn block_side(shape: &Shape, level: u8) -> f64 {
    voxel_size(shape, level) * BLOCK_SAMPLES as f64
}

pub(super) fn voxel_size(shape: &Shape, level: u8) -> f64 {
    shape.voxel * (1u64 << level) as f64
}
