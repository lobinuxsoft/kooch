//! [`OrbitInput`] — the authored action a [`CameraOrbit`](super::CameraOrbit) reads its look from.
//!
//! Separate from the orbit itself because an orbit driven by a cutscene, a spline or an AI has
//! limits and a recentring but no player: the binding is what makes it one, and its absence is how a
//! camera says nobody is holding the stick.

use kooch_core::Guid;
use kooch_core::resource::Resources;
use kooch_ecs::Reflect;
use kooch_ecs::component::{Component, ComponentRegistry};
use kooch_input::actions::LoadedActions;
use kooch_input::backend::InputBackend;

use super::CameraOrbit;

/// Which action turns this camera. Beside [`CameraOrbit`], which says what the action *does*.
///
/// A reference, not a name: renaming `Look.inputaction` changes nothing, which is why an action
/// carries a guid of its own.
#[derive(Debug, Clone, Copy, PartialEq, Default, Reflect)]
#[reflect(category = "Camera")]
pub struct OrbitInput {
    /// Where the player wants to look. A `vector2` action — mouse motion and the right stick
    /// together, read as a rate whichever answers. Unset turns nothing.
    #[reflect(asset = "kooch_input::actions::action::Action")]
    pub look: Option<Guid>,
    /// The button that asks the camera back behind the target now — R3 on a pad. A `button` action;
    /// unset asks for nothing, and the automatic return on [`CameraOrbit`] is unaffected either way.
    ///
    /// The bridge writes whether it is **held**; the press is [`CameraOrbit`]'s to derive, so a
    /// script asking the same way gets the same one-per-press.
    #[reflect(asset = "kooch_input::actions::action::Action")]
    pub recentre: Option<Guid>,
}

impl Component for OrbitInput {}

/// What one bound camera is being asked for this step.
struct Asked {
    entity: kooch_ecs::entity::Entity,
    look: glam::Vec2,
    recentre: bool,
}

/// Fills every [`CameraOrbit`] that names an action with this frame's values.
///
/// Zero is written as readily as anything else: letting go is what starts a recentring, and skipping
/// the write would leave the camera turning on its own.
pub fn read_orbit_input(resources: &mut Resources) {
    let asked = asked(resources);
    if asked.is_empty() {
        return;
    }
    let Some(registry) = resources.get_mut::<ComponentRegistry>() else {
        return;
    };
    let Some(orbits) = registry.get_cpu_mut::<CameraOrbit>() else {
        return;
    };
    for ask in &asked {
        if let Some(orbit) = orbits.get_mut(ask.entity) {
            orbit.look = ask.look;
            orbit.recentre_now = ask.recentre;
        }
    }
}

/// What each bound camera is being asked for, read before the orbits are written.
fn asked(resources: &Resources) -> Vec<Asked> {
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return Vec::new();
    };
    let Some(bindings) = registry.get_cpu::<OrbitInput>() else {
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
        .filter(|(_, binding)| binding.look.is_some() || binding.recentre.is_some())
        .map(|(&entity, binding)| {
            let look = loaded
                .evaluate(binding.look, &**backend)
                .map(|value| value.vector2())
                .unwrap_or_default();
            Asked {
                entity,
                look,
                recentre: loaded
                    .evaluate(binding.recentre, &**backend)
                    .is_some_and(|value| value.pressed),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
