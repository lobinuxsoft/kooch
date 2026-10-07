//! Impulse and shake: a landing or an explosion moves the camera (#1255).
//!
//! Cinemachine's shape, in three pieces. A **source** emits an impulse somewhere in the world, a
//! **listener** on a vcam picks it up with a falloff by distance, and the signal decays over its
//! duration along a curve.
//!
//! 🔴 **A curve, never a random walk.** Two players hit by the same explosion feel the same shake,
//! and a recorded one plays back. The four shapes are Cinemachine's own keyframes — see [`shape`].
//!
//! 🔴 **Applied after the pose and never into it.** The brain adds the offset where it transposes,
//! beside the dutch and for the same reason: a shake fed back into the rig would be damped, which
//! means remembered, and the camera would drift toward wherever the last explosion pushed it. That
//! place only exists because #1413 split computing from transposing.

#[cfg(feature = "physics")]
pub mod collision;

use glam::Vec3;
use kooch_core::resource::Resources;
use kooch_ecs::Reflect;
use kooch_ecs::component::{Component, ComponentRegistry};
use kooch_ecs::entity::Entity;
use kooch_ecs::impulse::{Impulse, shape};
use kooch_ecs::reflect::FieldRange;

/// A vcam that is shaken by impulses it can hear.
///
/// 🔴 Its own component rather than a field on the vcam, because its absence is the statement that
/// this camera is not shaken — the rule #1397 settled.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Impulse")]
pub struct ImpulseListener {
    /// Scales everything heard. `0` is deaf without removing the component.
    #[reflect(range = GAIN_RANGE)]
    pub gain: f32,
    /// Which channels this one hears.
    #[reflect(layers)]
    pub channels: u32,
    /// Shake along the camera's own axes rather than the world's, so a sideways knock is sideways
    /// on screen whichever way the camera faces.
    pub camera_space: bool,
}

const GAIN_RANGE: FieldRange = FieldRange {
    min: 0.0,
    max: 10.0,
    step: 0.05,
};

impl Default for ImpulseListener {
    fn default() -> Self {
        Self {
            gain: 1.0,
            channels: u32::MAX,
            camera_space: true,
        }
    }
}

impl Component for ImpulseListener {}

/// One impulse in flight.
#[derive(Debug, Clone, Copy)]
struct Live {
    at: Vec3,
    amplitude: Vec3,
    shape: u32,
    duration: f32,
    elapsed: f32,
    radius: f32,
    dissipation: f32,
    channels: u32,
}

/// Every impulse still sounding.
///
/// 🔴 A flat `Vec` walked per listener rather than a map: there are a handful in flight at once and
/// they are read every frame by every listener, so the walk is the cheap direction.
#[derive(Debug, Default)]
pub struct Impulses(Vec<Live>);

impl Impulses {
    /// Takes one impulse off the bus.
    pub fn hear(&mut self, impulse: Impulse) {
        if !(impulse.duration > 0.0) {
            return;
        }
        self.0.push(Live {
            at: impulse.at,
            amplitude: impulse.amplitude,
            shape: impulse.shape,
            duration: impulse.duration,
            elapsed: 0.0,
            radius: impulse.radius.max(0.0),
            dissipation: impulse.dissipation.max(0.0),
            channels: impulse.channels,
        });
    }

    /// What a listener at `at` feels right now, in world space.
    pub fn heard(&self, at: Vec3, listener: ImpulseListener) -> Vec3 {
        if !(listener.gain > 0.0) {
            return Vec3::ZERO;
        }
        let mut total = Vec3::ZERO;
        for live in &self.0 {
            if live.channels & listener.channels == 0 {
                continue;
            }
            let strength = dissipated(at.distance(live.at), live.radius, live.dissipation);
            if !(strength > 0.0) {
                continue;
            }
            let signal = shape::at(live.shape, live.elapsed / live.duration);
            total += live.amplitude * signal * strength;
        }
        total * listener.gain
    }

    /// Advances every impulse and drops the ones that have finished.
    fn step(&mut self, dt: f32) {
        if !(dt > 0.0) {
            return;
        }
        for live in &mut self.0 {
            live.elapsed += dt;
        }
        self.0.retain(|live| live.elapsed < live.duration);
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// How much of an impulse survives `distance` away: all of it inside the radius, nothing past the
/// dissipation, and a smooth fall between.
///
/// 🔴 Smoothstep rather than linear. A linear falloff has a corner at the radius, and a camera
/// crossing it changes how hard it is being shaken in one frame — which reads as a second, smaller
/// impulse firing.
fn dissipated(distance: f32, radius: f32, dissipation: f32) -> f32 {
    if distance <= radius {
        return 1.0;
    }
    if !(dissipation > 0.0) {
        return 0.0;
    }
    let t = ((distance - radius) / dissipation).clamp(0.0, 1.0);
    let fade = 1.0 - t;
    fade * fade * (3.0 - 2.0 * fade)
}

/// Takes everything published this frame, then advances what is in flight.
///
/// 🔴 The FIXED list. An impulse is fired by whatever noticed the thing that caused it — a solver
/// contact, a landing — on the fixed cadence, and the frame list loses five arrivals in six at
/// 360 fps (#1312).
pub fn step_impulses(resources: &mut Resources) {
    let arrived: Vec<Impulse> = resources
        .get::<kooch_core::event::Events<Impulse>>()
        .map(|events| events.read_fixed().copied().collect())
        .unwrap_or_default();
    let dt = crate::plugin::frame_dt(resources);
    if let Some(impulses) = resources.get_mut::<Impulses>() {
        for impulse in arrived {
            impulses.hear(impulse);
        }
        impulses.step(dt);
    }
}

/// What the camera at `at` is shaken by, in the axes the listener asked for.
pub(crate) fn offset_for(
    resources: &Resources,
    vcam: Entity,
    at: Vec3,
    rotation: glam::Quat,
) -> Vec3 {
    let Some(impulses) = resources.get::<Impulses>() else {
        return Vec3::ZERO;
    };
    if impulses.is_empty() {
        return Vec3::ZERO;
    }
    let Some(listener) = resources
        .get::<ComponentRegistry>()
        .and_then(|registry| registry.get_cpu::<ImpulseListener>()?.get(vcam).copied())
    else {
        return Vec3::ZERO;
    };
    let heard = impulses.heard(at, listener);
    match listener.camera_space {
        true => rotation * heard,
        false => heard,
    }
}

#[cfg(test)]
mod tests;
