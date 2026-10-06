//! [`ThirdPersonAim`] — what a shoulder camera is actually aiming at: Cinemachine's
//! `CinemachineThirdPersonAim` (#1370).
//!
//! 🔴 The problem is the shoulder's own. With a lateral offset the camera stands beside the head, so
//! the view axis and the line the character shoots along are **not the same line** — a reticle at the
//! centre of the screen and a projectile from the character's hands disagree, and the disagreement
//! grows with the distance.
//!
//! So there are two points, as there are in Cinemachine: where the **camera** is looking, and what
//! the **character** would hit shooting at it. Both are published on the component, because in an
//! ECS what "returning a value" means is writing it where a system can read it.

use glam::Vec3;
use kooch_core::resource::Resources;
use kooch_ecs::Reflect;
use kooch_ecs::component::{Component, ComponentRegistry};
use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::FieldRange;

/// Resolves what the vcam it sits on is aiming at. Beside a [`VirtualCamera`]; without the engine's
/// `physics` feature it answers the fallback point and nothing else.
///
/// [`VirtualCamera`]: crate::VirtualCamera
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Camera")]
pub struct ThirdPersonAim {
    /// Off publishes nothing and costs nothing.
    pub enabled: bool,
    /// Which collision groups the aim ray tests.
    ///
    /// 🔴 Groups, not Cinemachine's `IgnoreTag`: the engine filters by collision groups everywhere
    /// and the epic decided that once (#1250). The character's own body is excluded the way the
    /// [`Deoccluder`](crate::Deoccluder) excludes it, by handle rather than by name.
    #[reflect(layers)]
    pub aim_collision_filter: u32,
    /// How far to look for something to aim at. Hitting nothing, the point sits this far out.
    #[reflect(range = DISTANCE_RANGE)]
    pub aim_distance: f32,
    /// Aim along the **target's** facing rather than the camera's, so a shake does not move the
    /// reticle.
    ///
    /// 🔴 Cinemachine also re-orients the camera at Finalize to hold that point at centre screen.
    /// Still not here — but the reason changed. There IS a noise stage now (#1255), and the
    /// objection about a second owner of the rotation is answered by where the shake lives: the
    /// brain applies it while transposing, beside the dutch, so cancelling it there is part of the
    /// same single write rather than a second one. What is missing is only the work. Its own issue.
    pub noise_cancellation: bool,
    /// Where the camera's own view axis lands. Written every step; authoring it does nothing.
    #[reflect(skip)]
    pub looking_at: Vec3,
    /// What the character would hit shooting at [`looking_at`](Self::looking_at) — a different point,
    /// because the camera stands beside them.
    #[reflect(skip)]
    pub aim_target: Vec3,
}

const DISTANCE_RANGE: FieldRange = FieldRange {
    min: 1.0,
    max: 1000.0,
    step: 1.0,
};

impl Default for ThirdPersonAim {
    fn default() -> Self {
        Self {
            enabled: true,
            aim_collision_filter: u32::MAX,
            aim_distance: 200.0,
            noise_cancellation: true,
            looking_at: Vec3::ZERO,
            aim_target: Vec3::ZERO,
        }
    }
}

impl Component for ThirdPersonAim {}

/// Resolves every enabled aim, after the rig has placed the cameras it reads.
pub fn resolve_aims(resources: &mut Resources) {
    let resolved = aimed(resources);
    if resolved.is_empty() {
        return;
    }
    let Some(registry) = resources.get_mut::<ComponentRegistry>() else {
        return;
    };
    let Some(aims) = registry.get_cpu_mut::<ThirdPersonAim>() else {
        return;
    };
    for (entity, looking_at, aim_target) in resolved {
        if let Some(aim) = aims.get_mut(entity) {
            aim.looking_at = looking_at;
            aim.aim_target = aim_target;
        }
    }
}

/// What each aim resolves to, read before any of them is written.
fn aimed(resources: &Resources) -> Vec<(Entity, Vec3, Vec3)> {
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return Vec::new();
    };
    let Some(aims) = registry.get_cpu::<ThirdPersonAim>() else {
        return Vec::new();
    };
    let Some(vcams) = registry.get_cpu::<crate::VirtualCamera>() else {
        return Vec::new();
    };
    let pose = crate::plugin::poses(registry);
    let targets = registry.get_cpu::<crate::CameraTarget>();

    aims.iter()
        .filter(|(_, aim)| aim.enabled)
        .filter_map(|(&entity, aim)| {
            let vcam = vcams.get(entity)?;
            let (eye, rotation) = pose(entity)?;
            let group = crate::plugin::target_pose(targets, vcam.group, &pose)?;
            // The character's facing when a shake must not move the reticle, the camera's own when
            // the view is what aims.
            let forward = match aim.noise_cancellation {
                true => group.rotation * Vec3::NEG_Z,
                false => rotation * Vec3::NEG_Z,
            };
            let looking_at = aim.resolved(resources, eye, group.position, forward);
            let aim_target = aim.reached(resources, group.position, looking_at);
            Some((entity, looking_at, aim_target))
        })
        .collect()
}

impl ThirdPersonAim {
    /// Where the camera's view axis lands, ignoring anything between the camera and the character:
    /// the ray starts level with them, so their own back never answers it.
    pub fn resolved(&self, resources: &Resources, eye: Vec3, target: Vec3, forward: Vec3) -> Vec3 {
        let past = (target - eye).dot(forward).max(0.0);
        let from = eye + forward * past;
        let reach = (self.aim_distance - past).max(1.0);
        match cast(resources, from, forward, reach, self.aim_collision_filter) {
            Some(hit) => hit,
            None => from + forward * reach,
        }
    }

    /// What the character reaches shooting at `looking_at`. A second ray because the camera stands
    /// beside them: a wall the view clears may be one their shoulder does not.
    pub fn reached(&self, resources: &Resources, target: Vec3, looking_at: Vec3) -> Vec3 {
        let to = looking_at - target;
        let reach = to.length();
        match reach > 1e-4 {
            true => cast(
                resources,
                target,
                to / reach,
                reach,
                self.aim_collision_filter,
            )
            .unwrap_or(looking_at),
            false => looking_at,
        }
    }
}

/// The first thing `direction` meets within `reach`, or nothing.
#[cfg(feature = "physics")]
fn cast(
    resources: &Resources,
    from: Vec3,
    direction: Vec3,
    reach: f32,
    groups: u32,
) -> Option<Vec3> {
    use kooch_physics::backend::{InteractionMask, QueryFilter};

    let world = resources.get::<kooch_physics::PhysicsWorld>()?;
    let filter = QueryFilter {
        exclude: None,
        groups: InteractionMask {
            memberships: u32::MAX,
            filter: groups,
        },
        skip_sensors: true,
    };
    let hit = world.backend().query_ray(from, direction, reach, filter)?;
    Some(from + direction * hit.t)
}

/// Without a solver there is nothing to hit, and the point is the fallback.
#[cfg(not(feature = "physics"))]
fn cast(
    _resources: &Resources,
    _from: Vec3,
    _direction: Vec3,
    _reach: f32,
    _groups: u32,
) -> Option<Vec3> {
    None
}

#[cfg(test)]
mod tests;
