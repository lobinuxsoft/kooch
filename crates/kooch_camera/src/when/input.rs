//! [`WhenInput`] — the authored action a [`CameraWhen`](super::CameraWhen) is asked for by.

use kooch_core::Guid;
use kooch_core::resource::Resources;
use kooch_ecs::Reflect;
use kooch_ecs::component::{Component, ComponentRegistry};
use kooch_ecs::entity::Entity;
use kooch_input::actions::LoadedActions;
use kooch_input::backend::InputBackend;

use super::CameraWhen;

/// Which action asks for this camera. Beside [`CameraWhen`], which says what it is worth.
///
/// Separate because a camera asked for by a state machine, a trigger volume or a cutscene has a
/// worth but no button: the binding is what makes it the player's, and its absence is how a camera
/// says somebody else decides.
#[derive(Debug, Clone, Copy, PartialEq, Default, Reflect)]
#[reflect(category = "Camera")]
pub struct WhenInput {
    /// The `button` action that asks for this camera — `LT` for a shoulder view. Unset asks for
    /// nothing, and leaves `CameraWhen::asked` to whoever else writes it.
    #[reflect(asset = "kooch_input::actions::action::Action")]
    pub action: Option<Guid>,
}

impl Component for WhenInput {}

/// Fills every [`CameraWhen`] that names an action with whether it is held.
///
/// The hold, not the press: a toggle derives its own edge, so a script writing `asked` the same way
/// gets the same rule.
pub fn read_when_input(resources: &mut Resources) {
    let asked = asked(resources);
    if asked.is_empty() {
        return;
    }
    let Some(registry) = resources.get_mut::<ComponentRegistry>() else {
        return;
    };
    let Some(whens) = registry.get_cpu_mut::<CameraWhen>() else {
        return;
    };
    for (entity, held) in asked {
        if let Some(when) = whens.get_mut(entity) {
            when.asked = held;
        }
    }
}

/// Which bound cameras are being asked for, read before the conditions are written.
fn asked(resources: &Resources) -> Vec<(Entity, bool)> {
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return Vec::new();
    };
    let Some(bindings) = registry.get_cpu::<WhenInput>() else {
        return Vec::new();
    };
    let Some(backend) = resources.get::<Box<dyn InputBackend>>() else {
        return Vec::new();
    };
    let Some(loaded) = resources.get::<LoadedActions>() else {
        return Vec::new();
    };
    bindings
        .iter()
        .filter(|(_, binding)| binding.action.is_some())
        .map(|(&entity, binding)| {
            let held = loaded
                .evaluate(binding.action, &**backend)
                .is_some_and(|value| value.pressed);
            (entity, held)
        })
        .collect()
}

#[cfg(test)]
mod tests;
