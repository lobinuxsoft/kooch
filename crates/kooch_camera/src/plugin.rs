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

use crate::target::CameraTarget;
use crate::virtual_camera::{
    INACTIVE_ALWAYS, SETTLE_EPSILON, UP_GRAVITY, UP_TARGET, VirtualCamera, seed_reference,
    transported,
};

/// Which way is up for a virtual camera, resolved from its `up_mode`.
///
/// A rolling body is why this is not simply the target's rotation: a
/// character controller aligns itself to gravity and its up is the
/// answer, but a ball rolling by friction spins freely and its up points
/// wherever the last bounce left it. Asking the field is the only source
/// that is right for both.
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

/// Up is away from the gravity acting where the target is.
///
/// Delegates rather than recomputing: a camera that decides "up"
/// differently from the controller under it produces a horizon that
/// disagrees with the floor, and nothing in either file looks wrong.
#[cfg(feature = "gravity")]
fn gravity_up(resources: &Resources, target_pos: Vec3) -> Vec3 {
    kooch_gravity::gravity_up(resources, target_pos)
}

/// Without `kooch_gravity` there is no field to ask, so the mode is
/// world up. Authoring it still round-trips, which matters: a scene
/// saved by the editor must not lose the setting when opened by a build
/// that happens to omit the feature.
#[cfg(not(feature = "gravity"))]
fn gravity_up(_resources: &Resources, _target_pos: Vec3) -> Vec3 {
    Vec3::Y
}

/// The component without the system, for a host that authors camera
/// behaviour but does not run it.
///
/// The editor is that host: gameplay lives in the project's process, so
/// this side needs the fields to exist as data — to mirror, inspect and
/// draw — and must never move a camera with them. It has its own camera
/// and a vcam fighting it for the viewport would be unusable.
pub struct CameraComponentsPlugin;

impl Plugin for CameraComponentsPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(Stage::Startup, |resources: &mut Resources| {
            if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                registry.register_cpu_reflected::<VirtualCamera>();
                registry.register_cpu_reflected::<CameraTarget>();
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
        // `PostPhysics`, and the stage matters more than it looks.
        //
        // The renderer reads the camera's `GlobalTransform`, and
        // `EcsPlugin` propagates transforms in `PostUpdate` — registered
        // before any plugin of ours, so a vcam writing in `PostUpdate`
        // would land after propagation and show up a frame late. In the
        // fixed stages the solver has already moved the target and
        // propagation is still ahead, so the pose is current in the same
        // frame that produced it.
        //
        // It also means `dt` is the fixed step, which is what makes the
        // damping deterministic instead of frame-rate dependent.
        app.insert_resource(CameraBlend::default());
        app.insert_resource(HorizonFrames::default());
        app.add_system(Stage::PostPhysics, run_if_playing(drive_virtual_cameras));
    }

    fn name(&self) -> &str {
        "CameraPlugin"
    }
}

/// The yaw origin each vcam is measuring from, carried between frames.
///
/// # Why the Host holds this and not the vcam
///
/// It is not authored and it is not a setting — it is where the camera
/// happened to be pointing, which is state the same shape as
/// [`CameraBlend`]. Putting it on the component would serialise it into
/// every scene and show it in the Inspector as a vector nobody should
/// edit.
///
/// Rebuilt each frame from the vcams actually seen, so one that is
/// deleted or goes inert leaves nothing behind.
#[derive(Debug, Clone, Default)]
pub struct HorizonFrames {
    /// Per vcam: the up it last used, and the reference it carried.
    frames: std::collections::HashMap<Entity, (Vec3, Vec3)>,
}

impl HorizonFrames {
    /// This vcam's yaw origin on a new up, carried from the last one.
    ///
    /// A vcam with no history is seeded from a world axis, which is
    /// discontinuous — see `seed_reference`. That only matters between
    /// frames, and a first frame has none.
    fn carry(&self, entity: Entity, up: Vec3) -> Vec3 {
        match self.frames.get(&entity) {
            Some((last_up, reference)) => transported(*reference, *last_up, up),
            None => seed_reference(up),
        }
    }
}

/// What the Host remembers between frames to blend one handover.
///
/// Only the pose it is coming *from* and how far along it is. The pose
/// it is going to is recomputed every frame, because the winning vcam
/// keeps following its target while the blend runs — freezing the
/// destination would make the camera arrive where the target used to
/// be.
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

    /// Begins a handover from wherever the camera is right now.
    ///
    /// From the *camera's* pose, not the outgoing vcam's. Mid-blend the
    /// two are different, and starting from the vcam would snap back to
    /// a pose nobody has seen since the last handover — which is exactly
    /// the interruption case upstream needs a `tween_interrupted` signal
    /// to handle. Reading the visible pose handles it by construction.
    fn begin(&mut self, winner: Entity, from: (Vec3, glam::Quat), duration: f32) {
        self.active = Some(winner);
        self.from_pos = from.0;
        self.from_rot = from.1;
        self.elapsed = 0.0;
        self.duration = duration.max(0.0);
    }
}

/// Advances every live virtual camera, then hands the winner's pose to the camera.
///
/// Two steps, in the order phantom-camera's Host uses them: each virtual
/// camera works out where *it* wants to be, and then one of them is
/// elected and copied onto the camera that actually renders. Keeping the
/// vcam poses separate is what makes blending (#671 phase 3) a matter of
/// interpolating between two of them.
pub fn drive_virtual_cameras(resources: &mut Resources) {
    let (plan, horizons) = plan_vcam_poses(resources);
    resources.insert(horizons);
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
        // A different vcam won. Start from where the camera is now — and
        // on the very first frame there is nowhere to come from, so a
        // scene opens on its camera instead of flying in from wherever
        // the entity happened to be placed.
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

/// Slerp along the shorter arc.
///
/// `q` and `-q` are the same rotation, so without flipping one to match
/// the other a 1° handover can be interpolated as 359° of roll.
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

/// Where a group's members are, and which way the framing calls up.
///
/// The position is their weighted centre; with one member that is
/// exactly its position, which keeps the single-subject case identical
/// to what the old `target` reference produced.
///
/// The rotation is the **heaviest** member's, not a blend. Averaging
/// quaternions across a group has no meaning a player would recognise —
/// two characters facing each other would produce a camera up-vector
/// pointing sideways. One subject owns the orientation, and it is the
/// one the framing is most about.
fn target_pose(
    targets: Option<&kooch_ecs::component::ComponentStorage<CameraTarget>>,
    group: u32,
    pose_of: &impl Fn(Entity) -> Option<(Vec3, glam::Quat)>,
) -> Option<(Vec3, glam::Quat)> {
    let targets = targets?;
    let mut members: Vec<(Vec3, f32)> = Vec::new();
    let mut heaviest: Option<(f32, glam::Quat, u32)> = None;

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
            Some((weight, _, index)) => {
                target.weight > weight || (target.weight == weight && entity.index() < index)
            }
        };
        if better {
            heaviest = Some((target.weight, rotation, entity.index()));
        }
    }

    let centre = crate::target::weighted_centre(&members)?;
    let rotation = heaviest.map(|(_, rotation, _)| rotation)?;
    Some((centre, rotation))
}

/// Works out every vcam's pose without holding a borrow, because writing
/// a `Transform` needs the storage mutably and reading the target's pose
/// needs it shared.
fn plan_vcam_poses(resources: &Resources) -> (Vec<Pose>, HorizonFrames) {
    let carried = resources
        .get::<HorizonFrames>()
        .cloned()
        .unwrap_or_default();
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return (Vec::new(), carried);
    };
    let Some(vcams) = registry.get_cpu::<VirtualCamera>() else {
        return (Vec::new(), carried);
    };
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

        // A vcam on an entity that also renders, and is not rendering,
        // does nothing unless it asked to. Straight from
        // phantom-camera's `InactiveUpdateMode`, and the same lesson as
        // #656: work nobody sees is work not worth doing. A plain vcam
        // has no `PerspectiveCamera` at all and is always a candidate —
        // being unelected is what makes it cheap, not being invisible.
        if vcam.inactive_update != INACTIVE_ALWAYS
            && let Some(cam) = cameras.and_then(|s| s.get(entity))
            && !cam.active
        {
            continue;
        }

        // Nothing carries this vcam's tag, or everything that does has
        // zero weight: leave it where it is rather than snapping it to
        // the origin. A group that fills up next frame is picked up next
        // frame, with no resolving step in between.
        let Some((target_pos, target_rot)) = target_pose(targets, vcam.group, &pose_of) else {
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
        let (desired_pos, desired_rot) = vcam.desired_with(
            target_pos,
            target_rot,
            current.position,
            current.rotation,
            up,
            reference,
        );
        let position = vcam.damped(current.position, desired_pos, dt);
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
    (plan, horizons)
}

/// The virtual camera that drives the render camera this frame: highest priority, ties
/// broken on the lower entity index.
///
/// The tie-break is not cosmetic. Component storage has no iteration
/// order worth relying on, so "whichever came last" — which is what
/// upstream can afford inside an ordered scene tree — would hand the
/// camera to a different vcam on different frames and read as jitter.
fn elect(plan: &[Pose]) -> Option<(Entity, &Pose)> {
    plan.iter()
        .min_by_key(|pose| (-pose.priority, pose.entity.index()))
        .map(|pose| (pose.entity, pose))
}

/// The camera the elected vcam should drive: the highest-priority active
/// one, which is the same rule the renderer uses to pick what it draws.
///
/// A vcam that is itself a camera drives itself, which is how a scene
/// with one camera and one vcam on it keeps working.
fn rendering_camera(resources: &Resources, winner: Entity) -> Option<Entity> {
    let registry = resources.get::<ComponentRegistry>()?;
    let Some(cameras) = registry.get_cpu::<PerspectiveCamera>() else {
        // No camera component anywhere: the vcam's own entity is all
        // there is to move.
        return Some(winner);
    };
    cameras
        .iter()
        .filter(|(_, cam)| cam.active)
        .min_by_key(|(entity, cam)| (-cam.priority, entity.index()))
        .map(|(entity, _)| *entity)
        .or(Some(winner))
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
