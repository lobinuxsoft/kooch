//! [`CameraPlugin`] — registers [`VirtualCamera`] and the Host that drives it.

use glam::Vec3;
use kooch_core::app::App;
use kooch_core::plugin::Plugin;
use kooch_core::resource::Resources;
use kooch_core::run_state::run_if_playing;
use kooch_core::stage::Stage;
use kooch_core::time::Time;
use kooch_ecs::GlobalTransform;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;
use kooch_ecs::perspective_camera::PerspectiveCamera;
use kooch_ecs::transform::Transform;

use crate::brain::CameraBrain;
use crate::framing::{CameraFraming, Lens, Tracked};
use crate::occlusion::Arms;
use crate::target::CameraTarget;
use crate::virtual_camera::{
    INACTIVE_ALWAYS, LOOK_AT_SIMPLE, SETTLE_EPSILON, UP_GRAVITY, UP_TARGET, VirtualCamera,
    seed_reference, transported,
};

/// Which way is up for a virtual camera, from its `up_mode`. Not the target's rotation: a rolling
/// ball's up points wherever the last bounce left it.
fn up_for(
    vcam: &VirtualCamera,
    resources: &Resources,
    target_pos: Vec3,
    target_rot: glam::Quat,
) -> Vec3 {
    match vcam.up_mode {
        UP_TARGET => target_rot * Vec3::Y,
        UP_GRAVITY => gravity_up(resources, target_pos),
        _ => Vec3::Y,
    }
}

/// Up is away from the gravity at the target — delegated, so the camera's horizon agrees with the
/// controller's floor.
#[cfg(feature = "gravity")]
fn gravity_up(resources: &Resources, target_pos: Vec3) -> Vec3 {
    kooch_gravity::gravity_up(resources, target_pos)
}

/// Without `kooch_gravity` there is no field, so up is world up; the setting still round-trips so a
/// scene keeps it.
#[cfg(not(feature = "gravity"))]
fn gravity_up(_resources: &Resources, _target_pos: Vec3) -> Vec3 {
    Vec3::Y
}

/// The component without the system, for the editor: it mirrors and inspects vcams but must never
/// let one fight its own viewport camera.
pub struct CameraComponentsPlugin;

impl Plugin for CameraComponentsPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(Stage::Startup, |resources: &mut Resources| {
            if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                registry.register_cpu_reflected::<VirtualCamera>();
                registry.register_cpu_reflected::<CameraTarget>();
                registry.register_cpu_reflected::<CameraBrain>();
                registry.register_cpu_reflected::<crate::occlusion::CameraCollision>();
                registry.register_cpu_reflected::<crate::framing::CameraFraming>();
            }
        });
    }

    fn name(&self) -> &str {
        "CameraComponentsPlugin"
    }
}

/// Registers [`VirtualCamera`] and drives it while playing.
pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugin(CameraComponentsPlugin);
        // `PostPhysics`: after the solver moves the target, before transforms propagate, so the
        // pose shows the same frame. `dt` is the fixed step, which keeps damping deterministic.
        app.insert_resource(CameraBlend::default());
        app.insert_resource(HorizonFrames::default());
        app.add_system(Stage::PostPhysics, run_if_playing(drive_virtual_cameras));
    }

    fn name(&self) -> &str {
        "CameraPlugin"
    }
}

/// The yaw origin each vcam measures from, carried between frames. Runtime state, not authored, so
/// it lives on the Host; rebuilt from the vcams seen each frame.
#[derive(Debug, Clone, Default)]
pub struct HorizonFrames {
    /// Per vcam: the up it last used, and the reference it carried.
    frames: std::collections::HashMap<Entity, (Vec3, Vec3)>,
}

impl HorizonFrames {
    /// This vcam's yaw origin on a new up, carried from the last; a first frame seeds it from a
    /// world axis (see `seed_reference`).
    fn carry(&self, entity: Entity, up: Vec3) -> Vec3 {
        match self.frames.get(&entity) {
            Some((last_up, reference)) => transported(*reference, *last_up, up),
            None => seed_reference(up),
        }
    }
}

/// What the Host remembers to blend one handover: the pose it comes from and its progress. The
/// destination is recomputed each frame because the winner keeps following.
#[derive(Debug, Clone, Copy, Default)]
pub struct CameraBlend {
    /// The vcam currently driving, if any.
    pub active: Option<Entity>,
    /// Where the render camera was when this handover started.
    from_pos: Vec3,
    from_rot: glam::Quat,
    /// Seconds since it started, and how many it was given.
    elapsed: f32,
    duration: f32,
}

impl CameraBlend {
    /// Whether a handover is still in progress.
    fn running(&self) -> bool {
        self.duration > 0.0 && self.elapsed < self.duration
    }

    /// Begins a handover from the camera's visible pose, not the outgoing vcam's, so interrupting a
    /// blend never snaps back.
    fn begin(&mut self, winner: Entity, from: (Vec3, glam::Quat), duration: f32) {
        self.active = Some(winner);
        self.from_pos = from.0;
        self.from_rot = from.1;
        self.elapsed = 0.0;
        self.duration = duration.max(0.0);
    }
}

/// Advances every live virtual camera, then hands the winner's pose to the camera. Keeping vcam
/// poses separate is what lets a blend interpolate between two.
pub fn drive_virtual_cameras(resources: &mut Resources) {
    let (plan, horizons, arms, tracked) = plan_vcam_poses(resources);
    resources.insert(horizons);
    resources.insert(arms);
    resources.insert(tracked);
    if plan.is_empty() {
        return;
    }
    apply_poses(resources, &plan);

    let Some((winner, pose)) = elect(&plan) else {
        return;
    };
    let Some(camera) = rendering_camera(resources, winner) else {
        return;
    };
    let (target_pos, target_rot) = (pose.position, pose.rotation);
    let (duration, curve, ease) = (pose.blend_duration, pose.blend_curve, pose.blend_ease);

    let dt = fixed_dt(resources);
    let mut blend = resources.get::<CameraBlend>().copied().unwrap_or_default();

    let (position, rotation) = if blend.active == Some(winner) {
        blend.elapsed += dt;
        if blend.running() {
            let t = crate::blend::eased(blend.elapsed / blend.duration, curve, ease);
            (
                blend.from_pos.lerp(target_pos, t),
                short_slerp(blend.from_rot, target_rot, t),
            )
        } else {
            (target_pos, target_rot)
        }
    } else {
        // A different vcam won: start from where the camera is now. On the first frame there is
        // nothing to come from, so the scene opens on its camera.
        let from = camera_pose(resources, camera).unwrap_or((target_pos, target_rot));
        let duration = if blend.active.is_none() {
            0.0
        } else {
            duration
        };
        blend.begin(winner, from, duration);
        if blend.running() {
            (from.0, from.1)
        } else {
            (target_pos, target_rot)
        }
    };

    resources.insert(blend);
    apply_poses(
        resources,
        &[Pose {
            entity: camera,
            position,
            rotation,
            priority: 0,
            blend_duration: 0.0,
            blend_curve: 0,
            blend_ease: 0,
        }],
    );
}

/// The fixed step, or a 60 Hz stand-in when there is no clock.
fn fixed_dt(resources: &Resources) -> f32 {
    resources
        .get::<Time>()
        .map(|time| time.fixed_delta_secs())
        .unwrap_or(1.0 / 60.0)
}

/// Where the render camera is right now.
fn camera_pose(resources: &Resources, camera: Entity) -> Option<(Vec3, glam::Quat)> {
    let registry = resources.get::<ComponentRegistry>()?;
    let transform = registry.get_cpu::<Transform>()?.get(camera)?;
    Some((transform.position, transform.rotation))
}

#[cfg(test)]
mod blend_tests;
#[cfg(test)]
mod brain_tests;
#[cfg(test)]
mod framing_tests;

/// Slerp along the shorter arc: `q` and `-q` are one rotation, and without matching them a 1°
/// handover can roll 359°.
fn short_slerp(from: glam::Quat, to: glam::Quat, t: f32) -> glam::Quat {
    let to = if from.dot(to) < 0.0 { -to } else { to };
    from.slerp(to, t).normalize()
}

/// A vcam and where it decided to be this frame.
struct Pose {
    entity: Entity,
    position: Vec3,
    rotation: glam::Quat,
    priority: i32,
    /// Copied off the vcam so electing one does not need a second lookup
    /// while the component storage is borrowed elsewhere.
    blend_duration: f32,
    blend_curve: u32,
    blend_ease: u32,
}

/// A group's weighted centre, and the heaviest member's rotation and entity. Averaging quaternions
/// across members has no meaning — two characters facing each other would tilt the camera sideways.
fn target_pose(
    targets: Option<&kooch_ecs::component::ComponentStorage<CameraTarget>>,
    group: u32,
    pose_of: &impl Fn(Entity) -> Option<(Vec3, glam::Quat)>,
) -> Option<(Vec3, glam::Quat, Entity)> {
    let targets = targets?;
    let mut members: Vec<(Vec3, f32)> = Vec::new();
    let mut heaviest: Option<(f32, glam::Quat, Entity)> = None;

    for (&entity, target) in targets.iter() {
        if target.group != group {
            continue;
        }
        let Some((position, rotation)) = pose_of(entity) else {
            continue;
        };
        members.push((position, target.weight));
        // Ties break on the lower entity index, for the same reason vcam
        // election does: component storage has no order to rely on, and
        // a tie resolved differently each frame reads as jitter.
        let better = match heaviest {
            None => true,
            Some((weight, _, held)) => {
                target.weight > weight || (target.weight == weight && entity.index() < held.index())
            }
        };
        if better {
            heaviest = Some((target.weight, rotation, entity));
        }
    }

    let centre = crate::target::weighted_centre(&members)?;
    let (_, rotation, entity) = heaviest?;
    Some((centre, rotation, entity))
}

/// Works out every vcam's pose without holding a borrow, because writing
/// a `Transform` needs the storage mutably and reading the target's pose
/// needs it shared.
fn plan_vcam_poses(resources: &Resources) -> (Vec<Pose>, HorizonFrames, Arms, Tracked) {
    let carried = resources
        .get::<HorizonFrames>()
        .cloned()
        .unwrap_or_default();
    let carried_arms = resources.get::<Arms>().cloned().unwrap_or_default();
    let carried_tracked = resources.get::<Tracked>().cloned().unwrap_or_default();
    let mut arms = Arms::default();
    let mut tracked = Tracked::default();
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return (Vec::new(), carried, carried_arms, carried_tracked);
    };
    let Some(vcams) = registry.get_cpu::<VirtualCamera>() else {
        return (Vec::new(), carried, carried_arms, carried_tracked);
    };
    let framings = registry.get_cpu::<CameraFraming>();
    let lens = lens(resources, registry);
    let cameras = registry.get_cpu::<PerspectiveCamera>();
    let transforms = registry.get_cpu::<Transform>();
    let globals = registry.get_cpu::<GlobalTransform>();
    let targets = registry.get_cpu::<CameraTarget>();

    let dt = fixed_dt(resources);

    // A target's world pose. `GlobalTransform` first so a target parented
    // to something moving is followed where it actually is, not where its
    // local offset says.
    let pose_of = |entity: Entity| -> Option<(Vec3, glam::Quat)> {
        if let Some(global) = globals.and_then(|storage| storage.get(entity)) {
            let (_, rotation, translation) = global.matrix.to_scale_rotation_translation();
            return Some((translation, rotation));
        }
        transforms
            .and_then(|s| s.get(entity))
            .map(|t| (t.position, t.rotation))
    };

    let mut plan = Vec::new();
    let mut horizons = HorizonFrames::default();
    for (&entity, vcam) in vcams.iter() {
        if vcam.is_inert() {
            continue;
        }

        // A vcam on an entity that renders but is not rendering does nothing unless it asked to
        // (phantom-camera's `InactiveUpdateMode`, #656). A plain vcam is always a candidate.
        if vcam.inactive_update != INACTIVE_ALWAYS
            && let Some(cam) = cameras.and_then(|s| s.get(entity))
            && !cam.active
        {
            continue;
        }

        // Nothing carries this vcam's tag, or every weight is zero: leave it in place rather than
        // snapping to the origin.
        let Some((target_pos, target_rot, target_entity)) =
            target_pose(targets, vcam.group, &pose_of)
        else {
            continue;
        };
        let Some(current) = transforms.and_then(|s| s.get(entity)) else {
            continue;
        };

        let up = up_for(vcam, resources, target_pos, target_rot);
        // Carried, not derived: a yaw origin built from `up` alone has a
        // pole, and a target rolling over it swings the camera half a
        // turn. See `seed_reference`.
        let reference = carried.carry(entity, up);
        horizons.frames.insert(entity, (up, reference));
        let framing = framings
            .and_then(|framings| framings.get(entity))
            .filter(|framing| framing.enabled);
        // The rig follows the tracked point, not the target, and aims so that point sits where the
        // framing puts it. A first framed step starts on the target: centred when it goes live.
        let (followed, aim) = match framing {
            Some(framing) => {
                let from = carried_tracked.of(entity).unwrap_or(target_pos);
                let depth = (from - current.position).dot(current.rotation * -Vec3::Z);
                let depth = if depth > 0.01 { depth } else { vcam.distance };
                let point = framing.follow(from, target_pos, current.rotation, depth, lens, dt);
                tracked.set(entity, point);
                (
                    point,
                    Some(framing.aim(point, current.rotation, depth, lens)),
                )
            }
            None => (target_pos, None),
        };
        let (desired_pos, desired_rot) = vcam.desired_with(
            followed,
            target_rot,
            current.position,
            current.rotation,
            up,
            reference,
        );
        let desired_rot = match aim {
            Some(aim) if vcam.look_at == LOOK_AT_SIMPLE => {
                crate::virtual_camera::look_at(desired_pos, aim, up, reference)
            }
            _ => desired_rot,
        };
        // From where the rig had the camera before any wall, not from where the wall put it: the
        // damping is the rig's, and a return is the collision's to time. A framed rig is not
        // damped: its soft zone is the easing, and a second one would move the zones off screen.
        let from = carried_arms.free_of(entity).unwrap_or(current.position);
        let position = match framing {
            Some(_) => desired_pos,
            None => vcam.damped(from, desired_pos, dt),
        };
        // After the damping, so a wall pulls the camera in at once rather than at the damping's
        // pace (#1251).
        let position = crate::occlusion::held(
            resources,
            entity,
            target_pos,
            Some(target_entity),
            position,
            (&carried_arms, &mut arms),
            dt,
        );
        // Damped too, because `up` is not a constant any more: crossing
        // between two gravity fields rotates the whole basis, and
        // snapping that in one frame throws the horizon over.
        let rotation = vcam.damped_rotation(current.rotation, desired_rot, dt);

        plan.push(Pose {
            entity,
            position,
            rotation,
            priority: vcam.priority,
            blend_duration: vcam.blend_duration,
            blend_curve: vcam.blend_curve,
            blend_ease: vcam.blend_ease,
        });
    }
    (plan, horizons, arms, tracked)
}

/// The lens every vcam is seen through: the driven camera's field of view over the last rendered
/// aspect. A vcam frames one screen, and that is the one.
fn lens(resources: &Resources, registry: &ComponentRegistry) -> Lens {
    let fov = registry
        .get_cpu::<PerspectiveCamera>()
        .and_then(|cameras| {
            let brains = registry.get_cpu::<CameraBrain>();
            cameras
                .iter()
                .filter(|(entity, cam)| {
                    cam.active && brains.is_some_and(|brains| brains.get(**entity).is_some())
                })
                .min_by_key(|(entity, cam)| (-cam.priority, entity.index()))
                .map(|(_, cam)| cam.fov)
        })
        .unwrap_or(PerspectiveCamera::default().fov);
    let aspect = resources
        .get::<kooch_ecs::ViewAspect>()
        .copied()
        .unwrap_or_default();
    Lens::new(fov, aspect.0)
}

/// The virtual camera driving the render camera: highest priority, ties to the lower entity index.
/// Stable on purpose — storage order is not, and an unstable winner reads as jitter.
fn elect(plan: &[Pose]) -> Option<(Entity, &Pose)> {
    plan.iter()
        .min_by_key(|pose| (-pose.priority, pose.entity.index()))
        .map(|pose| (pose.entity, pose))
}

/// The camera the elected vcam drives: the one carrying a live [`CameraBrain`], and nothing else. A
/// vcam that is itself the only camera-less entity drives itself.
///
/// 🔴 Declared, never guessed (#1221). "The highest-priority camera" was the renderer's own rule
/// while a frame was one camera; with a stack it hands the rig to whatever overlay outranks the
/// base. A rig that moves a camera nobody pointed it at is worse than one that moves nothing — the
/// second says so.
fn rendering_camera(resources: &Resources, winner: Entity) -> Option<Entity> {
    let registry = resources.get::<ComponentRegistry>()?;
    let Some(cameras) = registry.get_cpu::<PerspectiveCamera>() else {
        // No camera component anywhere: the vcam's own entity is all
        // there is to move.
        return Some(winner);
    };
    let brains = registry.get_cpu::<CameraBrain>();
    let driven = cameras
        .iter()
        .filter(|(entity, cam)| {
            cam.active
                && brains
                    .is_some_and(|brains| brains.get(**entity).is_some_and(|brain| brain.enabled))
        })
        .min_by_key(|(entity, cam)| (-cam.priority, entity.index()))
        .map(|(entity, _)| *entity);
    if driven.is_none() {
        // Once: this is an authoring mistake, and a rig that says it every frame buries the log it
        // is trying to be read in.
        static SAID: std::sync::Once = std::sync::Once::new();
        SAID.call_once(|| {
            tracing::warn!(
                "a virtual camera is live and no camera carries an enabled CameraBrain: \
                 nothing is being driven. Add Camera Brain to the camera this rig is for."
            );
        });
    }
    driven
}

/// Writes the planned poses, skipping the ones that have arrived.
fn apply_poses(resources: &mut Resources, plan: &[Pose]) {
    let Some(registry) = resources.get_mut::<ComponentRegistry>() else {
        return;
    };
    let Some(transforms) = registry.get_cpu_mut::<Transform>() else {
        return;
    };
    for step in plan {
        let Some(transform) = transforms.get_mut(step.entity) else {
            continue;
        };
        // Below the floor it has arrived. Writing anyway would dirty a
        // transform to propagate and mirror on every frame of a scene
        // that is standing still.
        if step
            .position
            .abs_diff_eq(transform.position, SETTLE_EPSILON)
            && step
                .rotation
                .abs_diff_eq(transform.rotation, SETTLE_EPSILON)
        {
            continue;
        }
        transform.position = step.position;
        transform.rotation = step.rotation;
    }
}
