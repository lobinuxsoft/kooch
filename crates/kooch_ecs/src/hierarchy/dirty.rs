//! What moved since the last propagation pass: the roots of the subtrees whose `Transform` or
//! `Parent` changed (#1859). Found by comparing against last pass's copy, so no writer has to report
//! its edits — a writer that forgot would leave a stale world silently.

use crate::component::ComponentRegistry;
use crate::entity::Entity;
use crate::transform::Transform;

use super::global_transform::GlobalTransform;
use super::parent::Parent;

/// A chain deeper than this is treated as a cycle: the full pass handles it.
const MAX_DEPTH: u32 = 256;

/// Last pass's local pose and parent per entity index; `generation` tells a reused index apart.
#[derive(Default)]
pub struct PropagationCache {
    generation: Vec<u32>,
    pose: Vec<[u32; 10]>,
    parent: Vec<Entity>,
    /// Scratch: which indices changed this pass.
    moved: Vec<bool>,
}

/// The roots to republish, or `None` when only a full pass is right: the first pass, or a moved
/// entity that has never been given a `GlobalTransform` (that needs an archetype move).
pub fn moved_roots(
    registry: &ComponentRegistry,
    cache: &mut PropagationCache,
) -> Option<Vec<Entity>> {
    let transforms = registry.get_cpu::<Transform>()?;
    let parents = registry.get_cpu::<Parent>();
    let globals = registry.get_cpu::<GlobalTransform>()?;
    let first = cache.generation.is_empty();
    let parent_of = |entity: Entity| {
        parents
            .as_ref()
            .and_then(|storage| storage.get(entity))
            .map_or(Entity::INVALID, |parent| parent.entity)
    };

    let mut moved = Vec::new();
    for (&entity, transform) in transforms.iter() {
        let index = entity.index() as usize;
        if index >= cache.generation.len() {
            let len = index + 1;
            cache.generation.resize(len, u32::MAX);
            cache.pose.resize(len, [0; 10]);
            cache.parent.resize(len, Entity::INVALID);
            cache.moved.resize(len, false);
        }
        let pose = bits(transform);
        let parent = parent_of(entity);
        let (Some(generation), Some(last), Some(above)) = (
            cache.generation.get_mut(index),
            cache.pose.get_mut(index),
            cache.parent.get_mut(index),
        ) else {
            continue;
        };
        if *generation == entity.generation() && *last == pose && *above == parent {
            continue;
        }
        *generation = entity.generation();
        *last = pose;
        *above = parent;
        moved.push(entity);
    }
    if first {
        return None;
    }
    if moved.iter().any(|entity| globals.get(*entity).is_none()) {
        return None;
    }

    for entity in &moved {
        if let Some(flag) = cache.moved.get_mut(entity.index() as usize) {
            *flag = true;
        }
    }
    // A moved entity under a moved ancestor is republished by the ancestor's subtree.
    let mut roots = Vec::with_capacity(moved.len());
    for &entity in &moved {
        let mut above = parent_of(entity);
        let mut depth = 0;
        let mut covered = false;
        while above.is_valid() {
            depth += 1;
            if depth > MAX_DEPTH {
                break;
            }
            if cache
                .moved
                .get(above.index() as usize)
                .copied()
                .unwrap_or(false)
            {
                covered = true;
                break;
            }
            above = parent_of(above);
        }
        if depth > MAX_DEPTH {
            roots.clear();
            break;
        }
        if !covered {
            roots.push(entity);
        }
    }
    for entity in &moved {
        if let Some(flag) = cache.moved.get_mut(entity.index() as usize) {
            *flag = false;
        }
    }
    if roots.is_empty() && !moved.is_empty() {
        return None;
    }
    Some(roots)
}

/// The pose bit for bit: `Transform` has no `PartialEq`.
fn bits(t: &Transform) -> [u32; 10] {
    let [px, py, pz] = t.position.to_array();
    let [rx, ry, rz, rw] = t.rotation.to_array();
    let [sx, sy, sz] = t.scale.to_array();
    [px, py, pz, rx, ry, rz, rw, sx, sy, sz].map(f32::to_bits)
}
