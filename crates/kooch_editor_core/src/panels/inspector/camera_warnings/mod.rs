//! Inspector warnings for camera rig components that sit where nothing will read them.

use kooch_ecs::entity::Entity;

use crate::state::EntityDisplayInfo;

/// The components the rig reads off the entity carrying the `VirtualCamera`, and nowhere else.
const OF_THE_RIG: &[&str] = &["CameraLookahead", "CameraFraming", "CameraCollision"];

/// A rig component on an entity with no `VirtualCamera`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Orphan(&'static str);

impl Orphan {
    /// One line for the panel; the explanation is on hover.
    pub(super) fn summary(self) -> String {
        format!("{} does nothing on this entity", self.0)
    }

    pub(super) fn message(self) -> String {
        format!(
            "The rig reads {} off the entity that carries the Virtual Camera, and this entity \
             has none — so nothing here is ever read, whatever these fields say. Move the \
             component onto the Virtual Camera, or remove it. A Camera Brain is not a rig: it \
             picks which virtual camera drives the camera and how it blends, nothing else.",
            self.0,
        )
    }
}

/// Which rig components on `entity` have no virtual camera to be read by.
pub(super) fn warnings_for(entity: Entity, entities: &[EntityDisplayInfo]) -> Vec<Orphan> {
    let Some(info) = entities.iter().find(|e| e.entity == entity) else {
        return Vec::new();
    };
    if has_component(info, "VirtualCamera") {
        return Vec::new();
    }
    OF_THE_RIG
        .iter()
        .filter(|name| has_component(info, name))
        .map(|name| Orphan(name))
        .collect()
}

/// Matched by `short_name`, which is what the display snapshot carries — a remote client has no
/// Rust type for a project's components, so the name is all there is on either side of the wire.
fn has_component(info: &EntityDisplayInfo, name: &str) -> bool {
    info.components
        .iter()
        .any(|component| component.short_name == name)
}

#[cfg(test)]
mod tests;
