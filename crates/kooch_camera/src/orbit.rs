//! [`CameraOrbit`] — turning a third-person vcam from a rate, and easing its yaw back behind the
//! target when nobody asks.

use glam::{Vec2, Vec3};
use kooch_core::resource::Resources;
use kooch_ecs::Reflect;
use kooch_ecs::component::{Component, ComponentRegistry};
use kooch_ecs::entity::Entity;

use crate::target::CameraTarget;
use crate::virtual_camera::{VirtualCamera, eased};

#[cfg(feature = "input")]
pub mod input;

/// Turning a third-person vcam: what a full stick does to its `yaw` and `pitch`, how far the pitch
/// may go, and where the yaw returns to when the player lets go.
///
/// `look` is written every frame — by gameplay, or by
/// `OrbitInput` from an authored action. The component asks for a
/// rate rather than reading a device, the way `Facing` asks a character for a direction.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Camera")]
pub struct CameraOrbit {
    /// Written **every frame**: a rate, not an angle. Right turns the view right, up looks up.
    pub look: Vec2,
    /// Degrees a second at a full stick, per axis. The mouse reaches it at whatever speed its
    /// binding's `Scale` calls a full stick.
    pub speed: Vec2,
    /// Whether the horizontal axis turns the other way.
    /// 🔴 The sign lives here and the magnitude in `speed`: a negative speed would be a second way
    /// to say the same thing, which is the shape of #1333.
    pub invert_yaw: bool,
    /// The same for the vertical axis — the one players actually ask for.
    pub invert_pitch: bool,
    /// How far up it may look, in degrees below the horizon; negative is above it.
    pub pitch_min: f32,
    /// How far down. Past about 80 the camera stands over the target and the yaw stops meaning
    /// anything.
    pub pitch_max: f32,
    /// Whether the yaw returns behind the target **on its own**, once nobody has looked for
    /// `recentre_wait` seconds — and **stays** there until the camera is moved. The camera tails the
    /// target, as Cinemachine's does: its axis rests at `Center`, and `Center` is behind the target.
    /// Off by default, which is what a game that never had it expects; writable at runtime, as
    /// Cinemachine's `Recentering.Enabled` is.
    /// 🔴 The single owner of that switch. `recentre_wait > 0` used to mean it too, and a magic
    /// value doing a switch's job is the shape of #1333 — it also cost the authored wait to say off.
    #[reflect(alias = "recentre")]
    pub auto_recentre: bool,
    /// A return being asked for — by an `OrbitInput`'s button, R3 on a pad, or by a script. Written
    /// every frame like `look`, and it starts a return whatever `recentre_wait` says and whether or
    /// not `auto_recentre` is on, as Cinemachine's `TriggerRecentering` does.
    ///
    /// Unlike the automatic one, a request **ends when it has recentred**: it is one return, not a
    /// mode the camera sits in (#1350).
    ///
    /// 🔴 **The press, not the hold**: held down it asks once. The edge is derived here rather than
    /// by whoever writes the field, so it holds for every caller and not just for the input bridge.
    pub recentre_now: bool,
    /// Seconds without a look before the automatic return starts.
    pub recentre_wait: f32,
    /// Seconds a return takes, leaving a hundredth of the turn — exponential, like every other
    /// easing here. Not a window: the automatic return has no end but a look, and a manual one ends
    /// where it arrives.
    pub recentre_time: f32,
    /// Seconds since the last look. Not authored: it is the state of a thumb, and a scene that
    /// stored it would load mid-turn.
    #[reflect(skip)]
    pub idle: f32,
    /// Whether a return asked for by hand is still on its way. Cleared where it arrives, or by a
    /// look — the automatic one needs no such flag, since `idle` already says whether it is live.
    #[reflect(skip)]
    pub returning_now: bool,
    /// Last step's `recentre_now`, so a request is heard as a press. Per entity, because two players
    /// holding one action are not one press — the same reason `Sprint` keeps `was_wanted`.
    #[reflect(skip)]
    pub was_asked: bool,
}

impl Default for CameraOrbit {
    fn default() -> Self {
        Self {
            look: Vec2::ZERO,
            speed: Vec2::splat(180.0),
            invert_yaw: false,
            invert_pitch: false,
            pitch_min: -30.0,
            pitch_max: 70.0,
            auto_recentre: false,
            recentre_now: false,
            recentre_wait: 1.0,
            recentre_time: 1.0,
            idle: 0.0,
            returning_now: false,
            was_asked: false,
        }
    }
}

impl Component for CameraOrbit {}

impl CameraOrbit {
    /// Degrees to turn this step, per axis, inverts applied. Right turns the view right, which is
    /// the orbit going the other way round the target; up looks up, and the pitch counts down from
    /// the horizon — so both axes subtract by default.
    fn asked(&self, dt: f32) -> Vec2 {
        let sign = Vec2::new(
            if self.invert_yaw { 1.0 } else { -1.0 },
            if self.invert_pitch { 1.0 } else { -1.0 },
        );
        self.look * self.speed.abs() * sign * dt
    }

    /// Whether a return asked for by hand is live this step: still on its way, or being asked for
    /// now — the press, not the hold.
    fn asked_return(&self) -> bool {
        self.returning_now || (self.recentre_now && !self.was_asked)
    }

    /// Whether the yaw is being returned this step, by either mode.
    ///
    /// 🔴 The two modes end differently on purpose (#1350). The automatic one is a **mode**: live for
    /// as long as nobody looks, so the camera tails the target — which is what Cinemachine does,
    /// where the axis rests at `Center` and `Center` is behind the target. A manual one is **one
    /// return**: it ends where it arrives.
    ///
    /// 🔴 The only place the question is answered. Asked in two, neither could be tested: taking
    /// the rule out of one left the other holding the camera still, and the sabotage passed.
    pub fn returning(&self, dt: f32) -> bool {
        self.asked_return() || (self.auto_recentre && self.idle + dt >= self.recentre_wait)
    }

    /// This step's angles, and the state to carry into the next.
    ///
    /// `behind` is the yaw that puts the camera behind the target, when there is a target to return
    /// to. A look cancels whatever is in flight: the hand outranks both the timer and the button.
    pub fn stepped(&self, yaw: f32, pitch: f32, behind: Option<f32>, dt: f32) -> Orbited {
        if self.look != Vec2::ZERO {
            let asked = self.asked(dt);
            return Orbited {
                yaw: yaw + asked.x,
                pitch: (pitch + asked.y).clamp(self.pitch_min, self.pitch_max),
                idle: 0.0,
                returning_now: false,
                was_asked: self.recentre_now,
            };
        }

        let idle = self.idle + dt;
        let asked = self.asked_return();
        let Some(wanted) = behind.filter(|_| self.returning(dt)) else {
            return Orbited {
                yaw,
                pitch,
                idle,
                returning_now: asked,
                was_asked: self.recentre_now,
            };
        };
        let wanted = nearest(yaw, wanted);
        let turned = eased(yaw, wanted, self.recentre_time, dt);
        Orbited {
            yaw: turned,
            pitch,
            idle,
            // A manual return is over where it arrives. The automatic one holds the camera there.
            returning_now: asked && (wanted - turned).abs() > ARRIVED,
            was_asked: self.recentre_now,
        }
    }
}

/// Degrees from behind at which a return asked for by hand is done. An exponential ease leaves a
/// hundredth of the gap after `recentre_time` and never exactly none, so the arrival is declared.
const ARRIVED: f32 = 0.25;

/// What one step of an orbit leaves: the angles to write on the vcam, and the state to carry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Orbited {
    pub yaw: f32,
    pub pitch: f32,
    pub idle: f32,
    pub returning_now: bool,
    pub was_asked: bool,
}

/// The yaw that puts the camera behind `facing`, in degrees from `reference` around `up` — the same
/// frame the spring arm swings in, so under gravity it is the local horizon that counts and not
/// world up.
pub fn behind(facing: Vec3, up: Vec3, reference: Vec3) -> Option<f32> {
    let up = up.try_normalize()?;
    let flat = |direction: Vec3| (direction - up * direction.dot(up)).try_normalize();
    let from = flat(reference)?;
    // The camera sits opposite where the target looks, and the arm's angle measures where it sits.
    let to = flat(-facing)?;
    let turn = from.angle_between(to) * from.cross(to).dot(up).signum();
    Some(turn.to_degrees())
}

/// The angle equal to `wanted` nearest `yaw`: yaw accumulates past a full turn, and easing towards
/// the raw value would take the long way round.
fn nearest(yaw: f32, wanted: f32) -> f32 {
    let delta = (wanted - yaw).rem_euclid(360.0);
    yaw + if delta > 180.0 { delta - 360.0 } else { delta }
}

/// One vcam's turn this step.
struct Turn {
    entity: Entity,
    orbited: Orbited,
}

/// Turns every orbiting vcam by its `look`, before the rig swings the arm they set.
///
/// 🔴 A system and not a [`RigStage`](crate::rig::RigStage): `yaw` and `pitch` are the vcam's own
/// fields, and a stage holds the vcam shared on purpose — the invariant #1331 exists to keep.
pub fn orbit_cameras(resources: &mut Resources) {
    let turns = planned(resources);
    if turns.is_empty() {
        return;
    }
    let Some(registry) = resources.get_mut::<ComponentRegistry>() else {
        return;
    };
    if let Some(vcams) = registry.get_cpu_mut::<VirtualCamera>() {
        for turn in &turns {
            if let Some(vcam) = vcams.get_mut(turn.entity) {
                vcam.yaw = turn.orbited.yaw;
                vcam.pitch = turn.orbited.pitch;
            }
        }
    }
    let Some(orbits) = registry.get_cpu_mut::<CameraOrbit>() else {
        return;
    };
    for turn in &turns {
        if let Some(orbit) = orbits.get_mut(turn.entity) {
            orbit.idle = turn.orbited.idle;
            orbit.returning_now = turn.orbited.returning_now;
            orbit.was_asked = turn.orbited.was_asked;
        }
    }
}

/// What each orbit decided, planned before anything is written: the target's pose is read from the
/// same registry the vcam is written to.
fn planned(resources: &Resources) -> Vec<Turn> {
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return Vec::new();
    };
    let Some(orbits) = registry.get_cpu::<CameraOrbit>() else {
        return Vec::new();
    };
    let Some(vcams) = registry.get_cpu::<VirtualCamera>() else {
        return Vec::new();
    };
    let dt = crate::plugin::fixed_dt(resources);
    let carried = resources.get::<crate::rig::RigMemory>();
    let targets = registry.get_cpu::<CameraTarget>();
    let pose_of = crate::plugin::poses(registry);

    orbits
        .iter()
        .filter_map(|(&entity, orbit)| {
            let vcam = vcams.get(entity)?;
            // Only a return needs to know where the target faces; turning does not, so a vcam whose
            // group carries nothing still answers the stick.
            let wanted = orbit
                .returning(dt)
                .then(|| {
                    let target = crate::plugin::target_pose(targets, vcam.group, &pose_of)?;
                    let up =
                        crate::plugin::up_for(vcam, resources, target.position, target.rotation);
                    let reference = carried.map_or_else(
                        || crate::virtual_camera::seed_reference(up),
                        |memory| memory.horizons.carry(entity, up),
                    );
                    behind(target.rotation * Vec3::NEG_Z, up, reference)
                })
                .flatten();

            Some(Turn {
                entity,
                orbited: orbit.stepped(vcam.yaw, vcam.pitch, wanted, dt),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests;
