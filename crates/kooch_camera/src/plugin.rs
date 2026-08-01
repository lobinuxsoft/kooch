//! [`CameraPlugin`] — registers [`CameraRig`] and the Host that drives it.

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

use crate::rig::{CameraRig, INACTIVE_ALWAYS, SETTLE_EPSILON};

/// The component without the system, for a host that authors camera
/// behaviour but does not run it.
///
/// The editor is that host: gameplay lives in the project's process, so
/// this side needs the fields to exist as data — to mirror, inspect and
/// draw — and must never move a camera with them. It has its own camera
/// and a rig fighting it for the viewport would be unusable.
pub struct CameraComponentsPlugin;

impl Plugin for CameraComponentsPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(Stage::Startup, |resources: &mut Resources| {
            if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                registry.register_cpu_reflected::<CameraRig>();
            }
        });
    }

    fn name(&self) -> &str {
        "CameraComponentsPlugin"
    }
}

/// Registers [`CameraRig`] and drives it while playing.
pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugin(CameraComponentsPlugin);
        // `PostPhysics`, and the stage matters more than it looks.
        //
        // The renderer reads the camera's `GlobalTransform`, and
        // `EcsPlugin` propagates transforms in `PostUpdate` — registered
        // before any plugin of ours, so a rig writing in `PostUpdate`
        // would land after propagation and show up a frame late. In the
        // fixed stages the solver has already moved the target and
        // propagation is still ahead, so the pose is current in the same
        // frame that produced it.
        //
        // It also means `dt` is the fixed step, which is what makes the
        // damping deterministic instead of frame-rate dependent.
        app.add_system(Stage::PostPhysics, run_if_playing(drive_camera_rigs));
    }

    fn name(&self) -> &str {
        "CameraPlugin"
    }
}

/// Advances every live rig, then hands the winner's pose to the camera.
///
/// Two steps, in the order phantom-camera's Host uses them: each virtual
/// camera works out where *it* wants to be, and then one of them is
/// elected and copied onto the camera that actually renders. Keeping the
/// vcam poses separate is what makes blending (#671 phase 3) a matter of
/// interpolating between two of them.
pub fn drive_camera_rigs(resources: &mut Resources) {
    let plan = plan_rig_poses(resources);
    if plan.is_empty() {
        return;
    }
    apply_poses(resources, &plan);

    if let Some((winner, pose)) = elect(&plan)
        && let Some(camera) = rendering_camera(resources, winner)
    {
        apply_poses(
            resources,
            &[Pose {
                entity: camera,
                position: pose.position,
                rotation: pose.rotation,
                priority: 0,
            }],
        );
    }
}

/// A vcam and where it decided to be this frame.
struct Pose {
    entity: Entity,
    position: Vec3,
    rotation: glam::Quat,
    priority: i32,
}

/// Works out every vcam's pose without holding a borrow, because writing
/// a `Transform` needs the storage mutably and reading the target's pose
/// needs it shared.
fn plan_rig_poses(resources: &Resources) -> Vec<Pose> {
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return Vec::new();
    };
    let Some(rigs) = registry.get_cpu::<CameraRig>() else {
        return Vec::new();
    };
    let cameras = registry.get_cpu::<PerspectiveCamera>();
    let transforms = registry.get_cpu::<Transform>();
    let globals = registry.get_cpu::<GlobalTransform>();

    let dt = resources
        .get::<Time>()
        .map(|time| time.fixed_delta_secs())
        .unwrap_or(1.0 / 60.0);

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
    for (&vcam, rig) in rigs.iter() {
        if rig.is_inert() {
            continue;
        }

        // A rig on an entity that also renders, and is not rendering,
        // does nothing unless it asked to. Straight from
        // phantom-camera's `InactiveUpdateMode`, and the same lesson as
        // #656: work nobody sees is work not worth doing. A plain vcam
        // has no `PerspectiveCamera` at all and is always a candidate —
        // being unelected is what makes it cheap, not being invisible.
        if rig.inactive_update != INACTIVE_ALWAYS
            && let Some(cam) = cameras.and_then(|s| s.get(vcam))
            && !cam.active
        {
            continue;
        }

        let Some(target) = rig.target.and_then(|reference| reference.entity()) else {
            continue;
        };
        // A target that was despawned, or that a scene never resolved,
        // leaves the vcam where it is rather than snapping it to the
        // origin.
        let Some((target_pos, target_rot)) = pose_of(target) else {
            continue;
        };
        let Some(current) = transforms.and_then(|s| s.get(vcam)) else {
            continue;
        };

        let (desired_pos, rotation) =
            rig.desired(target_pos, target_rot, current.position, current.rotation);
        let position = rig.damped(current.position, desired_pos, dt);

        plan.push(Pose {
            entity: vcam,
            position,
            rotation,
            priority: rig.priority,
        });
    }
    plan
}

/// The rig that drives the camera this frame: highest priority, ties
/// broken on the lower entity index.
///
/// The tie-break is not cosmetic. Component storage has no iteration
/// order worth relying on, so "whichever came last" — which is what
/// upstream can afford inside an ordered scene tree — would hand the
/// camera to a different rig on different frames and read as jitter.
fn elect(plan: &[Pose]) -> Option<(Entity, &Pose)> {
    plan.iter()
        .min_by_key(|pose| (-pose.priority, pose.entity.index()))
        .map(|pose| (pose.entity, pose))
}

/// The camera the elected rig should drive: the highest-priority active
/// one, which is the same rule the renderer uses to pick what it draws.
///
/// A vcam that is itself a camera drives itself, which is how a scene
/// with one camera and one rig on it keeps working.
fn rendering_camera(resources: &Resources, winner: Entity) -> Option<Entity> {
    let registry = resources.get::<ComponentRegistry>()?;
    let Some(cameras) = registry.get_cpu::<PerspectiveCamera>() else {
        // No camera component anywhere: the rig's own entity is all
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
