//! [`CameraPlugin`] — registers [`VirtualCamera`] and the Host that drives it.

use glam::Vec3;
use kooch_core::app::App;
use kooch_core::plugin::Plugin;
use kooch_core::resource::Resources;
use kooch_core::run_state::run_if_playing;
use kooch_core::schedule::Order;
use kooch_core::stage::Stage;
use kooch_core::time::Time;
use kooch_ecs::GlobalTransform;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;
use kooch_ecs::perspective_camera::PerspectiveCamera;
use kooch_ecs::transform::Transform;

use crate::brain::CameraBrain;
use crate::frame::CameraFrame;
use crate::framing::Lens;
use crate::rig::{CameraRig, RigMemory, RigStep};
use crate::target::{CameraTarget, GroupPose};
use crate::virtual_camera::{
    INACTIVE_ALWAYS, SETTLE_EPSILON, UP_GRAVITY, UP_TARGET, VirtualCamera,
};

/// Which way is up for a virtual camera, from its `up_mode`. Not the target's rotation: a rolling
/// ball's up points wherever the last bounce left it.
pub(crate) fn up_for(
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
                registry.register_cpu_reflected::<crate::occlusion::Deoccluder>();
                registry.register_cpu_reflected::<crate::framing::RotationComposer>();
                registry.register_cpu_reflected::<crate::lookahead::CameraLookahead>();
                registry.register_cpu_reflected::<crate::orbit::CameraOrbit>();
                registry.register_cpu_reflected::<crate::when::CameraWhen>();
                #[cfg(feature = "input")]
                {
                    registry.register_cpu_reflected::<crate::orbit::input::OrbitInput>();
                    registry.register_cpu_reflected::<crate::when::input::WhenInput>();
                }
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
        app.insert_resource(RigMemory::default());
        app.insert_resource(CameraRig::standard());
        // Before anything reads a duration, and before an author can edit a field a switch overrode.
        app.add_system(Stage::First, crate::virtual_camera::migrate_damping_switch);
        app.add_system(Stage::First, crate::virtual_camera::report_moved_blends);
        app.add_system(Stage::First, crate::rig::report_orphans);
        // Declared, not left to registration order: the orbit writes the `yaw` the rig's Body
        // swings the arm by, so a rig that ran first would swing last frame's angle (#392).
        #[cfg(feature = "input")]
        {
            app.add_ordered(
                Stage::PostPhysics,
                Order::before("orbit_cameras"),
                run_if_playing(crate::orbit::input::read_orbit_input),
            );
            app.add_ordered(
                Stage::PostPhysics,
                Order::before("step_camera_whens"),
                run_if_playing(crate::when::input::read_when_input),
            );
        }
        // Before the plan the election reads, and after whoever wrote the condition.
        app.add_ordered(
            Stage::PostPhysics,
            Order::before("drive_virtual_cameras"),
            run_if_playing(crate::when::step_camera_whens),
        );
        app.add_ordered(
            Stage::PostPhysics,
            Order::before("drive_virtual_cameras"),
            run_if_playing(crate::orbit::orbit_cameras),
        );
        app.add_system(Stage::PostPhysics, run_if_playing(drive_virtual_cameras));
    }

    fn name(&self) -> &str {
        "CameraPlugin"
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
    let (plan, memory) = plan_vcam_poses(resources);
    resources.insert(memory);
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
    // 🔴 The brain's, not the incoming vcam's. A blend is between two of them, and asking one only
    // raises "which?" — the answer used to be "whichever is arriving", a convention (#1339).
    let (duration, curve, ease) = blend_settings(resources, camera);

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
        }],
    );
}

/// The fixed step, or a 60 Hz stand-in when there is no clock.
pub(crate) fn fixed_dt(resources: &Resources) -> f32 {
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
mod aim_tests;
#[cfg(test)]
mod blend_tests;
#[cfg(test)]
mod brain_tests;
#[cfg(test)]
mod framing_tests;
#[cfg(test)]
mod when_tests;

/// Slerp along the shorter arc: `q` and `-q` are one rotation, and without matching them a 1°
/// handover can roll 359°.
fn short_slerp(from: glam::Quat, to: glam::Quat, t: f32) -> glam::Quat {
    let to = if from.dot(to) < 0.0 { -to } else { to };
    from.slerp(to, t).normalize()
}

/// How the brain on `camera` blends between virtual cameras.
fn blend_settings(resources: &Resources, camera: Entity) -> (f32, u32, u32) {
    let brain = resources
        .get::<ComponentRegistry>()
        .and_then(|registry| registry.get_cpu::<CameraBrain>()?.get(camera).copied())
        .unwrap_or_default();
    (brain.blend_duration, brain.blend_curve, brain.blend_ease)
}

/// A vcam and where it decided to be this frame.
struct Pose {
    entity: Entity,
    position: Vec3,
    rotation: glam::Quat,
    priority: i32,
}

/// Where a vcam's group is this step, or `None` when nothing carries its tag and there is nothing
/// to follow.
pub(crate) fn target_pose(
    targets: Option<&kooch_ecs::component::ComponentStorage<CameraTarget>>,
    group: u32,
    pose_of: &impl Fn(Entity) -> Option<(Vec3, glam::Quat)>,
) -> Option<GroupPose> {
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
        // Ties break on the lower entity index, for the same reason vcam election does: component
        // storage has no order to rely on, and a tie resolved differently each frame reads as jitter.
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

    let position = crate::target::weighted_centre(&members)?;
    let (_, rotation, heaviest) = heaviest?;
    Some(GroupPose {
        position,
        rotation,
        heaviest,
    })
}

/// Runs the rig over every live vcam. Takes `&Resources` because writing a `Transform` needs the
/// storage mutably while reading the target's pose needs it shared, so the poses are planned first
/// and written after.
fn plan_vcam_poses(resources: &Resources) -> (Vec<Pose>, RigMemory) {
    let carried = resources.get::<RigMemory>().cloned().unwrap_or_default();
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return (Vec::new(), carried);
    };
    let Some(vcams) = registry.get_cpu::<VirtualCamera>() else {
        return (Vec::new(), carried);
    };
    let Some(rig) = resources.get::<CameraRig>() else {
        // Once: a rig with no stages moves nothing, and a rig that says so every frame buries it.
        static SAID: std::sync::Once = std::sync::Once::new();
        SAID.call_once(|| {
            tracing::warn!("no CameraRig is registered: no virtual camera can move anything");
        });
        return (Vec::new(), carried);
    };
    let lens = lens(resources, registry);
    let cameras = registry.get_cpu::<PerspectiveCamera>();
    let transforms = registry.get_cpu::<Transform>();
    let targets = registry.get_cpu::<CameraTarget>();
    let dt = fixed_dt(resources);

    let pose_of = poses(registry);

    let mut plan = Vec::new();
    let mut memory = RigMemory::default();
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

        // Nothing carries this vcam's tag, or every weight is zero: leave it where it is rather than
        // snapping to the origin.
        let Some(target) = target_pose(targets, vcam.group, &pose_of) else {
            continue;
        };
        let Some(current) = transforms.and_then(|s| s.get(entity)) else {
            continue;
        };

        let up = up_for(vcam, resources, target.position, target.rotation);
        let reference = carried.horizons.carry(entity, up);
        memory.horizons.set(entity, up, reference);
        // Where the rig had the camera before any wall, not where the wall put it: the damping is the
        // body's, and a return is the collision's to time.
        let previous = carried.arms.free_of(entity).unwrap_or(current.position);

        let mut step = RigStep {
            frame: CameraFrame::new(
                current.position,
                previous,
                current.rotation,
                target.position,
                lens,
            ),
            entity,
            vcam,
            target,
            up,
            reference,
            dt,
            resources,
            registry,
            carried: &carried,
            memory: &mut memory,
        };
        rig.run(&mut step);

        plan.push(Pose {
            entity,
            position: step.frame.position,
            rotation: step.frame.rotation,
            // 🔴 The authored number plus whatever a condition adds, never the condition's own: a
            // component that wrote `vcam.priority` would be a second owner of it, and letting go
            // would not give the authored value back (#1352).
            priority: vcam.priority + crate::when::boost_of(registry, entity),
        });
    }
    (plan, memory)
}

/// A target's world pose. `GlobalTransform` first, so a target parented to something moving is
/// followed where it actually is and not where its local offset says.
pub(crate) fn poses(
    registry: &ComponentRegistry,
) -> impl Fn(Entity) -> Option<(Vec3, glam::Quat)> + '_ {
    let transforms = registry.get_cpu::<Transform>();
    let globals = registry.get_cpu::<GlobalTransform>();
    move |entity| {
        if let Some(global) = globals.and_then(|storage| storage.get(entity)) {
            let (_, rotation, translation) = global.matrix.to_scale_rotation_translation();
            return Some((translation, rotation));
        }
        transforms
            .and_then(|storage| storage.get(entity))
            .map(|transform| (transform.position, transform.rotation))
    }
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
