//! The rig — the ordered list of stages a pose passes through, and what each one may decide
//! (#1331).
//!
//! 🔴 The order used to be the order of statements in one 200-line function, and a stage read
//! whatever local the stage above it happened to leave behind. Four bugs in a row were that same
//! shape: two things deciding one quantity. Here a stage is registered at a [`RigStage`], is handed
//! the pose, changes the single quantity it owns, and hands it on — so a new component (an orbit
//! input, a noise, a timeline) plugs in without anyone editing the loop.

use glam::Vec3;
use kooch_core::resource::Resources;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;

use crate::frame::CameraFrame;
use crate::framing::Tracked;
use crate::lookahead::Leads;
use crate::occlusion::Arms;
use crate::target::GroupPose;
use crate::virtual_camera::{VirtualCamera, seed_reference, transported};

/// Where a stage runs, and by running there what it is allowed to decide. Declared in order: a
/// stage never reads a later one's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RigStage {
    /// How far ahead of the target the rig looks. Owns `target` and `screen`.
    Lead,
    /// Where the camera stands. Owns `position` and `free`.
    Body,
    /// Moves `position` sideways, never in depth. Nothing of the engine's runs here since the
    /// framing became an aim (#1361); the slot stays because displacing the camera before the walls
    /// are considered is still a coherent thing for a project to want.
    Frame,
    /// The last word on `position`: a wall pulls the camera in.
    Collide,
    /// Where the camera looks. Owns `rotation`, and exactly one thing does: the vcam's `look_at`
    /// picks which, as Cinemachine's Rotation Control slot holds one component.
    Aim,
}

/// One stage's work on one vcam's pose.
pub type RigFn = fn(&mut RigStep);

/// The stages, in order. Each component's plugin registers its own, which is what makes "one owner
/// per quantity" a thing the rig is built out of rather than a comment.
#[derive(Debug, Clone, Default)]
pub struct CameraRig {
    stages: Vec<(RigStage, RigFn)>,
}

impl CameraRig {
    /// The stages a camera rig is made of, in the order a pose passes through them.
    pub fn standard() -> Self {
        let mut rig = Self::default();
        rig.add(RigStage::Lead, crate::lookahead::lead_stage);
        rig.add(RigStage::Body, crate::virtual_camera::body_stage);
        rig.add(RigStage::Collide, crate::occlusion::collide_stage);
        rig.add(RigStage::Aim, crate::virtual_camera::aim_stage);
        // Last at each stage, because an extension is the hand-made word over what the stage
        // decided — added after it, which is what `add` keeps in order.
        rig.add(RigStage::Lead, crate::extensions::after_lead);
        rig.add(RigStage::Body, crate::extensions::after_body);
        rig.add(RigStage::Frame, crate::extensions::after_frame);
        rig.add(RigStage::Collide, crate::extensions::after_collide);
        rig.add(RigStage::Aim, crate::extensions::after_aim);
        rig
    }

    /// Adds a stage, keeping the list in [`RigStage`] order. Two at the same stage run in the order
    /// they were added.
    pub fn add(&mut self, stage: RigStage, run: RigFn) {
        let at = self.stages.partition_point(|(other, _)| *other <= stage);
        self.stages.insert(at, (stage, run));
    }

    /// Runs every stage over one pose.
    pub fn run(&self, step: &mut RigStep) {
        for (_, run) in &self.stages {
            run(step);
        }
    }
}

/// Everything a stage may read, and the pose it is changing.
pub struct RigStep<'a> {
    /// The pose in flight. A stage writes the one quantity its [`RigStage`] owns.
    pub frame: CameraFrame,
    /// The vcam being posed.
    pub entity: Entity,
    /// How it was authored.
    pub vcam: &'a VirtualCamera,
    /// The real target: a group's centre, and the member whose rotation stands for the group.
    /// 🔴 Not [`CameraFrame::target`], which is the point being framed — a lead moves that one.
    pub target: GroupPose,
    /// Which way is up for this vcam.
    pub up: Vec3,
    /// The yaw origin it measures from, carried between steps.
    pub reference: Vec3,
    /// The fixed step, so damping is deterministic.
    pub dt: f32,
    /// For a stage that asks another crate a question, as the collision asks physics.
    pub resources: &'a Resources,
    /// For a stage that reads its own component off `entity`.
    pub registry: &'a ComponentRegistry,
    /// What the stages left last step.
    pub carried: &'a RigMemory,
    /// What they are leaving this one.
    pub memory: &'a mut RigMemory,
}

/// What the stages carry between steps. Runtime state, never authored, and rebuilt from the vcams
/// seen each step so a despawned one leaves nothing behind.
#[derive(Debug, Clone, Default)]
pub struct RigMemory {
    /// Each vcam's yaw origin.
    pub horizons: Horizons,
    /// Each vcam's arm: where the rig would have it, and any return in progress.
    pub arms: Arms,
    /// Where each rotation composer last saw its target.
    pub tracked: Tracked,
    /// The same for each position composer. Its own, because a vcam may carry both and one memory
    /// shared between two owners is the defect this rig keeps having.
    pub composed: Tracked,
    /// Each lead's offset.
    pub leads: Leads,
}

/// Which entities have already been told a rig component of theirs is read by nothing.
#[derive(Debug, Clone, Default)]
pub struct Orphans(std::collections::HashSet<Entity>);

/// Says once, per entity, that a rig component sits where nothing will read it.
///
/// 🔴 The Inspector says the same thing where the author is looking (#1342); this is for a packaged
/// build, which has no Inspector. A component that does nothing and says nothing is the worst of
/// both: it is tuned for hours and never read.
pub fn report_orphans(resources: &mut Resources) {
    let mut found: Vec<(Entity, &'static str)> = Vec::new();
    if let Some(registry) = resources.get::<ComponentRegistry>() {
        let vcams = registry.get_cpu::<VirtualCamera>();
        let posed = |entity: Entity| vcams.is_some_and(|vcams| vcams.get(entity).is_some());
        let mut sweep =
            |name: &'static str, entities: Vec<Entity>, reads: &dyn Fn(Entity) -> bool| {
                found.extend(
                    entities
                        .into_iter()
                        .filter(|entity| !reads(*entity))
                        .map(|entity| (entity, name)),
                );
            };
        sweep(
            "CameraLookahead",
            entities_of::<crate::CameraLookahead>(registry),
            &posed,
        );
        // 🔴 Every mode is a component now, so "is this read" is "is the vcam a rig at all" —
        // the component's own presence says which mode it is (#1397).
        sweep(
            "RotationComposer",
            entities_of::<crate::RotationComposer>(registry),
            &posed,
        );
        sweep(
            "OrbitalFollow",
            entities_of::<crate::OrbitalFollow>(registry),
            &posed,
        );
        sweep(
            "ThirdPersonFollow",
            entities_of::<crate::ThirdPersonFollow>(registry),
            &posed,
        );
        sweep(
            "PositionComposer",
            entities_of::<crate::PositionComposer>(registry),
            &posed,
        );
        sweep("Follow", entities_of::<crate::Follow>(registry), &posed);
        sweep(
            "HardLockToTarget",
            entities_of::<crate::HardLockToTarget>(registry),
            &posed,
        );
        sweep(
            "HardLookAt",
            entities_of::<crate::HardLookAt>(registry),
            &posed,
        );
        sweep("PanTilt", entities_of::<crate::PanTilt>(registry), &posed);
        sweep(
            "RotateWithFollowTarget",
            entities_of::<crate::RotateWithFollowTarget>(registry),
            &posed,
        );
        sweep(
            "CameraOrbit",
            entities_of::<crate::orbit::CameraOrbit>(registry),
            &posed,
        );
        sweep(
            "CameraOffset",
            entities_of::<crate::CameraOffset>(registry),
            &posed,
        );
        sweep(
            "CameraRecomposer",
            entities_of::<crate::CameraRecomposer>(registry),
            &posed,
        );
        sweep(
            "ThirdPersonAim",
            entities_of::<crate::ThirdPersonAim>(registry),
            &posed,
        );
        sweep(
            "CameraWhen",
            entities_of::<crate::when::CameraWhen>(registry),
            &posed,
        );
        // A binding is read off the orbit it fills, not off the vcam: an `OrbitInput` alone names an
        // action nothing turns.
        #[cfg(feature = "input")]
        {
            let orbits = registry.get_cpu::<crate::orbit::CameraOrbit>();
            let turning =
                |entity: Entity| orbits.is_some_and(|orbits| orbits.get(entity).is_some());
            sweep(
                "OrbitInput",
                entities_of::<crate::orbit::input::OrbitInput>(registry),
                &turning,
            );
            let whens = registry.get_cpu::<crate::when::CameraWhen>();
            let asked = |entity: Entity| whens.is_some_and(|whens| whens.get(entity).is_some());
            sweep(
                "WhenInput",
                entities_of::<crate::when::input::WhenInput>(registry),
                &asked,
            );
        }
    }

    // 🔴 Two composers on one vcam is two owners of where the target sits on screen: the body
    // slides to put it there and the aim turns to put it there, and they chase each other.
    let mut both: Vec<Entity> = Vec::new();
    // 🔴 A shoulder under an aim that re-frames the target: measured, an offset of 0.6 moves the
    // character −0.117 of the screen under `Pan Tilt` and 0.000 under `Rotation Composer` or
    // `Hard Look At`. The rig is working and the aim is undoing it.
    let mut cancelled: Vec<Entity> = Vec::new();
    // 🔴 The case a smoke test found with nothing said: a vcam that names a target and carries no
    // body at all, or carries two. The component is the choice, so counting them is the check
    // `[DisallowMultipleComponent]` and an editor-managed slot do for Cinemachine (#1397).
    let mut miscounted: Vec<(Entity, usize, usize)> = Vec::new();
    if let Some(registry) = resources.get::<ComponentRegistry>()
        && let Some(vcams) = registry.get_cpu::<VirtualCamera>()
    {
        let shoulders = registry.get_cpu::<crate::ThirdPersonFollow>();
        for (&entity, vcam) in vcams.iter() {
            if !vcam.enabled {
                continue;
            }
            let bodies = crate::virtual_camera::bodies_on(registry, entity);
            let aims = crate::virtual_camera::aims_on(registry, entity);
            if bodies != 1 || aims > 1 {
                miscounted.push((entity, bodies, aims));
            }
            if crate::virtual_camera::one::<crate::PositionComposer>(registry, entity).is_some()
                && crate::framing::of(registry, entity).is_some()
            {
                both.push(entity);
            }
            let offset = shoulders
                .and_then(|bodies| bodies.get(entity))
                .is_some_and(|body| body.shoulder_offset != Vec3::ZERO);
            let reframed = crate::framing::of(registry, entity).is_some()
                || crate::virtual_camera::one::<crate::HardLookAt>(registry, entity).is_some();
            if offset && reframed {
                cancelled.push(entity);
            }
        }
    }

    let mut said = resources.get::<Orphans>().cloned().unwrap_or_default();
    for entity in cancelled {
        if !said.0.insert(entity) {
            continue;
        }
        tracing::warn!(
            target: "kooch_camera",
            entity = entity.index(),
            "a shoulder offset under an aim that re-frames the target: the aim decides where the \
             character sits on screen, so the shoulder only shifts the parallax. Pan Tilt is the \
             aim a shoulder rig is built on.",
        );
    }
    for (entity, bodies, aims) in miscounted {
        if !said.0.insert(entity) {
            continue;
        }
        tracing::warn!(
            target: "kooch_camera",
            entity = entity.index(),
            bodies,
            aims,
            "a virtual camera does not carry exactly one body: the component is what says where the \
             camera stands, so none means nothing places it and two means they disagree.",
        );
    }
    for entity in both {
        if !said.0.insert(entity) {
            continue;
        }
        tracing::warn!(
            target: "kooch_camera",
            entity = entity.index(),
            "a virtual camera composes in both slots: the body slides the target to where it belongs \
             and the aim turns it there, and they answer each other. Pick one.",
        );
    }
    for (entity, name) in found {
        if !said.0.insert(entity) {
            continue;
        }
        tracing::warn!(
            target: "kooch_camera",
            entity = entity.index(),
            component = name,
            "a camera rig component sits on an entity where nothing reads it",
        );
    }
    resources.insert(said);
}

/// Every entity carrying `C`.
fn entities_of<C: kooch_ecs::component::Component>(registry: &ComponentRegistry) -> Vec<Entity> {
    registry
        .get_cpu::<C>()
        .map(|storage| storage.iter().map(|(entity, _)| *entity).collect())
        .unwrap_or_default()
}

/// The yaw origin each vcam measures from, carried between steps.
#[derive(Debug, Clone, Default)]
pub struct Horizons {
    /// Per vcam: the up it last used, and the reference it carried.
    frames: std::collections::HashMap<Entity, (Vec3, Vec3)>,
}

impl Horizons {
    /// This vcam's yaw origin on a new up, carried from the last. A first step seeds it from a world
    /// axis: by the hairy ball theorem no reference derived from `up` alone is continuous, so one
    /// derived every step would swing the camera half a turn as a target rolls over a pole.
    pub fn carry(&self, vcam: Entity, up: Vec3) -> Vec3 {
        match self.frames.get(&vcam) {
            Some((last_up, reference)) => transported(*reference, *last_up, up),
            None => seed_reference(up),
        }
    }

    /// The up and reference this vcam last used, or `None` for one the rig has not planned.
    ///
    /// 🔴 For anything that has to show what the rig **did**, rather than work out what it would do:
    /// a gizmo that re-derived these would draw a rig that is not the one running (#1379).
    pub fn used(&self, vcam: Entity) -> Option<(Vec3, Vec3)> {
        self.frames.get(&vcam).copied()
    }

    /// Remembers what this vcam used, for the next step to carry from.
    pub fn set(&mut self, vcam: Entity, up: Vec3, reference: Vec3) {
        self.frames.insert(vcam, (up, reference));
    }
}

#[cfg(test)]
mod tests;
