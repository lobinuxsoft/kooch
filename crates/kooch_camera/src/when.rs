//! [`CameraWhen`] — a vcam that outranks the others while something holds.

use kooch_ecs::Reflect;
use kooch_ecs::component::{Component, ComponentRegistry};
use kooch_ecs::entity::Entity;

#[cfg(feature = "input")]
pub mod input;

/// Raises this vcam's priority while `asked`, so the brain elects it and blends to it.
///
/// 🔴 It **adds** to the authored `priority` rather than writing it: a component that overwrote the
/// field would be a second owner of it, and the authored value would not survive letting go.
///
/// `asked` is written every frame by whoever decides — a [`WhenInput`](input::WhenInput)'s button, a
/// state machine, a trigger volume, a script. A condition the engine never heard of is a project
/// system writing this field, not a change in here.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Camera")]
pub struct CameraWhen {
    /// Written **every frame**: whether this camera is being asked for now.
    pub asked: bool,
    /// Added to the vcam's `priority` while it counts. One more than the camera it has to beat is
    /// enough; the brain's blend does the rest.
    pub boost: i32,
    /// A press turns it on and another turns it off, instead of counting only while held.
    pub toggle: bool,
    /// Last step's `asked`, so a toggle hears presses rather than holds. Per entity, because two
    /// players holding one action are not one press — as `Sprint` keeps `was_wanted`.
    #[reflect(skip)]
    pub was_asked: bool,
    /// Whether the boost counts this step.
    #[reflect(skip)]
    pub on: bool,
}

impl Default for CameraWhen {
    fn default() -> Self {
        Self {
            asked: false,
            boost: 10,
            toggle: false,
            was_asked: false,
            on: false,
        }
    }
}

impl Component for CameraWhen {}

impl CameraWhen {
    /// Advances one step, exactly as [`Sprint::step`](kooch_character) does: held is while asked,
    /// toggled flips on each press.
    pub fn step(&mut self) {
        let pressed = self.asked && !self.was_asked;
        self.was_asked = self.asked;
        self.on = match self.toggle {
            true => self.on != pressed,
            false => self.asked,
        };
    }

    /// What this component adds to the vcam's priority this step.
    pub fn boost(&self) -> i32 {
        match self.on {
            true => self.boost,
            false => 0,
        }
    }
}

/// Advances every camera's condition, before the rig plans the poses the election picks from.
pub fn step_camera_whens(resources: &mut kooch_core::resource::Resources) {
    let Some(registry) = resources.get_mut::<ComponentRegistry>() else {
        return;
    };
    let Some(whens) = registry.get_cpu_mut::<CameraWhen>() else {
        return;
    };
    for (_, when) in whens.iter_mut() {
        when.step();
    }
}

/// What `entity`'s condition adds to its priority, or nothing when it carries none.
pub(crate) fn boost_of(registry: &ComponentRegistry, entity: Entity) -> i32 {
    registry
        .get_cpu::<CameraWhen>()
        .and_then(|whens| whens.get(entity))
        .map_or(0, CameraWhen::boost)
}

#[cfg(test)]
mod tests;
