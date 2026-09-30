//! Inspector warnings for camera rig components that sit where nothing will read them.

use kooch_ecs::entity::Entity;

use crate::state::EntityDisplayInfo;

/// Each camera component, the one that reads it, and — where the reader only reads it in one mode —
/// the field that has to say so. Anywhere else it is authored and inert.
const OF_THE_RIG: &[(&str, &str, Option<InMode>)] = &[
    ("CameraLookahead", "VirtualCamera", None),
    // A framing is the vcam's Rotation Control: on a vcam that aims some other way the whole
    // component does nothing, however long it was tuned for (#1361).
    (
        "RotationComposer",
        "VirtualCamera",
        Some(InMode {
            field: "look_at",
            value: kooch_camera::LOOK_AT_COMPOSED,
            asked: "Composed (framing)",
        }),
    ),
    ("Deoccluder", "VirtualCamera", None),
    (
        "PositionComposer",
        "VirtualCamera",
        Some(InMode {
            field: "follow",
            value: kooch_camera::FOLLOW_POSITION_COMPOSER,
            asked: "Position Composer",
        }),
    ),
    (
        "OrbitalFollow",
        "VirtualCamera",
        Some(InMode {
            field: "follow",
            value: kooch_camera::FOLLOW_ORBITAL,
            asked: "Orbital Follow",
        }),
    ),
    (
        "ThirdPersonFollow",
        "VirtualCamera",
        Some(InMode {
            field: "follow",
            value: kooch_camera::FOLLOW_SHOULDER,
            asked: "Third Person Follow",
        }),
    ),
    ("ThirdPersonAim", "VirtualCamera", None),
    ("CameraOffset", "VirtualCamera", None),
    ("CameraRecomposer", "VirtualCamera", None),
    ("CameraOrbit", "VirtualCamera", None),
    ("CameraWhen", "VirtualCamera", None),
    // A binding fills the component it is named for, so that is what has to be here — not the vcam.
    ("OrbitInput", "CameraOrbit", None),
    ("WhenInput", "CameraWhen", None),
];

/// The mode a reader has to be in to read a component at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct InMode {
    field: &'static str,
    value: u32,
    /// What the dropdown calls it, so the fix reads as the thing the author has to click.
    asked: &'static str,
}

/// A camera component on an entity without the one that reads it, or with a reader not asking for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Orphan {
    component: &'static str,
    reader: &'static str,
    /// Set when the reader is here and the mode is what is missing.
    unasked: Option<InMode>,
}

impl Orphan {
    /// One line for the panel; the explanation is on hover.
    pub(super) fn summary(self) -> String {
        format!("{} does nothing on this entity", self.component)
    }

    pub(super) fn message(self) -> String {
        let named = spaced(self.reader);
        if let Some(mode) = self.unasked {
            return format!(
                "{} is read only while the {}'s {} is \"{}\", and it is not — so nothing here is \
                 ever read, whatever these fields say. Set it, or remove the component.",
                self.component,
                named,
                spaced(mode.field),
                mode.asked,
            );
        }
        let mut message = format!(
            "{} is read off the entity that carries the {}, and this entity has none — so \
             nothing here is ever read, whatever these fields say. Move the component onto the \
             {}, or remove it.",
            self.component, named, named,
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

/// A name as the Inspector titles it: `VirtualCamera` is `Virtual Camera` there, and `look_at` is
/// `Look At`.
fn spaced(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 2);
    let mut fresh = true;
    for letter in name.chars() {
        if letter == '_' {
            out.push(' ');
            fresh = true;
            continue;
        }
        if letter.is_uppercase() && !out.is_empty() {
            out.push(' ');
        }
        out.extend(match fresh {
            true => letter.to_uppercase().collect::<Vec<_>>(),
            false => vec![letter],
        });
        fresh = false;
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
        .filter(|(component, ..)| has_component(info, component))
        .filter_map(
            |(component, reader, mode)| match has_component(info, reader) {
                false => Some(Orphan {
                    component,
                    reader,
                    unasked: None,
                }),
                // The reader is here; whether it asks for this is a field's answer, and a snapshot that
                // skipped the values cannot say. Silence beats a warning drawn from nothing.
                true => mode
                    .filter(|mode| asks(info, reader, mode) == Some(false))
                    .map(|mode| Orphan {
                        component,
                        reader,
                        unasked: Some(mode),
                    }),
            },
        )
        .collect()
}

/// Whether `reader` on this entity is in `mode`, or `None` where its values were not read.
fn asks(info: &EntityDisplayInfo, reader: &str, mode: &InMode) -> Option<bool> {
    let component = info
        .components
        .iter()
        .find(|c| c.short_name.as_ref() == reader)?;
    let values = component.fields.values()?;
    let (_, value) = values.iter().find(|(name, _)| name == mode.field)?;
    match value {
        kooch_ecs::reflect::ReflectValue::U32(at) => Some(*at == mode.value),
        _ => None,
    }
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
