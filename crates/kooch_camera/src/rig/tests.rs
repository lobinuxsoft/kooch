//! The rig is a list (#1331): the order comes from [`RigStage`], not from the order stages are
//! registered in.

use super::*;
use crate::framing::Lens;
use glam::Quat;
use kooch_ecs::allocator::EntityAllocator;

thread_local! {
    /// Which stages ran, in the order they ran.
    static RAN: std::cell::RefCell<Vec<RigStage>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn note(stage: RigStage) {
    RAN.with(|ran| ran.borrow_mut().push(stage));
}

fn ran() -> Vec<RigStage> {
    RAN.with(|ran| ran.borrow().clone())
}

fn lead(_: &mut RigStep) {
    note(RigStage::Lead);
}

fn body(_: &mut RigStep) {
    note(RigStage::Body);
}

fn collide(_: &mut RigStep) {
    note(RigStage::Collide);
}

fn aim(_: &mut RigStep) {
    note(RigStage::Aim);
}

/// Runs `rig` over one pose that nothing is following.
fn run(rig: &CameraRig) {
    let resources = Resources::new();
    let registry = ComponentRegistry::new();
    let vcam = VirtualCamera::default();
    let carried = RigMemory::default();
    let mut memory = RigMemory::default();
    let entity = EntityAllocator::new().spawn();
    let mut step = RigStep {
        frame: CameraFrame::new(
            Vec3::ZERO,
            Vec3::ZERO,
            Quat::IDENTITY,
            Vec3::ZERO,
            Lens::new(60.0, 16.0 / 9.0),
        ),
        entity,
        vcam: &vcam,
        target: GroupPose {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            heaviest: entity,
        },
        up: Vec3::Y,
        reference: Vec3::Z,
        dt: 1.0 / 60.0,
        resources: &resources,
        registry: &registry,
        carried: &carried,
        memory: &mut memory,
    };
    rig.run(&mut step);
}

#[test]
fn a_stage_runs_in_order() {
    let mut rig = CameraRig::default();
    rig.add(RigStage::Aim, aim);
    rig.add(RigStage::Lead, lead);
    rig.add(RigStage::Collide, collide);
    rig.add(RigStage::Body, body);
    run(&rig);
    assert_eq!(
        ran(),
        vec![
            RigStage::Lead,
            RigStage::Body,
            RigStage::Collide,
            RigStage::Aim
        ],
    );
}

#[test]
fn a_tie_keeps_its_arrival() {
    let mut rig = CameraRig::default();
    rig.add(RigStage::Body, body);
    rig.add(RigStage::Body, collide);
    rig.add(RigStage::Body, aim);
    run(&rig);
    assert_eq!(
        ran(),
        vec![RigStage::Body, RigStage::Collide, RigStage::Aim],
    );
}

/// A rig component is read off the vcam's entity, and nowhere else: on any other one it is tuned
/// for nothing, so the rig says so (#1342).
#[test]
fn an_orphan_is_reported() {
    use kooch_ecs::component::ComponentRegistry;

    let mut resources = Resources::new();
    let mut allocator = EntityAllocator::new();
    let mut registry = ComponentRegistry::new();
    registry.register_cpu_reflected::<VirtualCamera>();
    registry.register_cpu_reflected::<crate::CameraLookahead>();
    let (brain, rig) = (allocator.spawn(), allocator.spawn());
    let lookaheads = registry.get_cpu_mut::<crate::CameraLookahead>().unwrap();
    lookaheads.insert(brain, crate::CameraLookahead::default());
    lookaheads.insert(rig, crate::CameraLookahead::default());
    registry
        .get_cpu_mut::<VirtualCamera>()
        .unwrap()
        .insert(rig, VirtualCamera::default());
    resources.insert(registry);

    super::report_orphans(&mut resources);

    let said = resources.get::<Orphans>().expect("nothing was reported");
    assert!(said.0.contains(&brain), "the orphan went unsaid");
    assert!(
        !said.0.contains(&rig),
        "a component beside its vcam was flagged"
    );
}
