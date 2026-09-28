//! Inspector warnings for camera rig components that sit where nothing will read them.

use kooch_ecs::entity::Entity;

use crate::state::EntityDisplayInfo;

/// Each camera component and the one that reads it. Anywhere else it is authored and inert.
const OF_THE_RIG: &[(&str, &str)] = &[
    ("CameraLookahead", "VirtualCamera"),
    ("CameraFraming", "VirtualCamera"),
    ("CameraCollision", "VirtualCamera"),
    ("CameraOrbit", "VirtualCamera"),
    // A binding fills an orbit, so the orbit is what has to be here — not the vcam.
    ("OrbitInput", "CameraOrbit"),
];

/// A camera component on an entity without the one that reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Orphan {
    component: &'static str,
    reader: &'static str,
}

impl Orphan {
    /// One line for the panel; the explanation is on hover.
    pub(super) fn summary(self) -> String {
        format!("{} does nothing on this entity", self.component)
    }

    pub(super) fn message(self) -> String {
        let spaced = spaced(self.reader);
        let mut message = format!(
            "{} is read off the entity that carries the {}, and this entity has none — so \
             nothing here is ever read, whatever these fields say. Move the component onto the \
             {}, or remove it.",
            self.component, spaced, spaced,
        );
        if self.reader == "VirtualCamera" {
            message.push_str(
                " A Camera Brain is not a rig: it picks which virtual camera drives the camera \
                 and how it blends, nothing else.",
            );
        }
        message
    }
}

/// A component's name as the Inspector titles it: `VirtualCamera` is `Virtual Camera` there.
fn spaced(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 2);
    for (at, letter) in name.char_indices() {
        if at > 0 && letter.is_uppercase() {
            out.push(' ');
        }
        out.push(letter);
    }
    out
}

/// Which camera components on `entity` have nothing here to read them.
pub(super) fn warnings_for(entity: Entity, entities: &[EntityDisplayInfo]) -> Vec<Orphan> {
    let Some(info) = entities.iter().find(|e| e.entity == entity) else {
        return Vec::new();
    };
    OF_THE_RIG
        .iter()
        .filter(|(component, reader)| {
            has_component(info, component) && !has_component(info, reader)
        })
        .map(|(component, reader)| Orphan { component, reader })
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
