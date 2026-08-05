//! Recursive despawn helper — collects an entity and all its descendants.

use crate::entity::Entity;

use super::children::Children;

/// Collects an entity and all its descendants (via `Children` components).
pub fn collect_descendants(
    root: Entity,
    registry: &crate::component::ComponentRegistry,
) -> Vec<Entity> {
    let mut result = vec![root];
    let mut i = 0;
    while i < result.len() {
        let current = result[i];
        if let Some(storage) = registry.get_cpu::<Children>() {
            if let Some(children) = storage.get(current) {
                result.extend_from_slice(&children.entities);
            }
        }
        i += 1;
    }
    result
}
