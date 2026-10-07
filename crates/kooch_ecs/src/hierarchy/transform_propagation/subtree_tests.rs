//! 🔴 That it publishes the subtree, and that it publishes NOTHING ELSE. The second half is the
//! whole point — a pass that quietly republished the world would satisfy every behavioural test
//! here while costing exactly what this exists to avoid.

use super::*;
use crate::allocator::EntityAllocator;
use crate::component::ComponentRegistry;
use crate::transform::Transform;
use glam::Vec3;
use kooch_core::resource::Resources;

fn at(x: f32) -> Transform {
    Transform {
        position: Vec3::new(x, 0.0, 0.0),
        ..Default::default()
    }
}

fn global_x(resources: &Resources, entity: Entity) -> f32 {
    resources
        .get::<ComponentRegistry>()
        .unwrap()
        .get_cpu::<GlobalTransform>()
        .unwrap()
        .get(entity)
        .unwrap()
        .matrix
        .to_scale_rotation_translation()
        .2
        .x
}

fn moved(resources: &mut Resources, entity: Entity, x: f32) {
    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(transforms) = registry.get_cpu_mut::<Transform>()
    {
        transforms.insert(entity, at(x));
    }
}

/// A parent, its child, and an unrelated entity — all already published at the origin.
fn world() -> (Resources, Entity, Entity, Entity) {
    let mut resources = Resources::new();
    let mut allocator = EntityAllocator::new();
    let mut registry = ComponentRegistry::new();
    registry.register_cpu_reflected::<Transform>();
    registry.register_cpu_reflected::<GlobalTransform>();
    registry.register_cpu_reflected::<Parent>();
    registry.register_cpu_reflected::<Children>();

    let (body, child, other) = (allocator.spawn(), allocator.spawn(), allocator.spawn());
    let transforms = registry.get_cpu_mut::<Transform>().unwrap();
    transforms.insert(body, at(0.0));
    transforms.insert(child, at(1.0));
    transforms.insert(other, at(100.0));
    registry
        .get_cpu_mut::<Parent>()
        .unwrap()
        .insert(child, Parent { entity: body });
    registry.get_cpu_mut::<Children>().unwrap().insert(
        body,
        Children {
            entities: vec![child],
        },
    );
    let globals = registry.get_cpu_mut::<GlobalTransform>().unwrap();
    for entity in [body, child, other] {
        globals.insert(entity, GlobalTransform::default());
    }
    resources.insert(registry);
    (resources, body, child, other)
}

#[test]
fn a_root_and_its_child_are_published() {
    let (mut resources, body, child, _) = world();
    moved(&mut resources, body, 5.0);
    propagate_subtrees(&mut resources, &[body]);
    assert!((global_x(&resources, body) - 5.0).abs() < 1e-5);
    assert!(
        (global_x(&resources, child) - 6.0).abs() < 1e-5,
        "the child was left at {}, not 5 plus its own 1",
        global_x(&resources, child),
    );
}

/// 🔴 The point of the whole thing: everything else is untouched.
#[test]
fn nothing_else_is_touched() {
    let (mut resources, body, _, other) = world();
    moved(&mut resources, body, 5.0);
    propagate_subtrees(&mut resources, &[body]);
    assert_eq!(
        global_x(&resources, other),
        0.0,
        "an unrelated entity was republished, so this is a full pass wearing a disguise",
    );
}

/// A root under a parent that did not move builds on the parent's PUBLISHED global, rather than
/// recomputing it — recomputing is the full pass this avoids.
#[test]
fn a_parented_root_builds_on_what_is_published() {
    let (mut resources, body, child, _) = world();
    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(globals) = registry.get_cpu_mut::<GlobalTransform>()
    {
        globals.insert(
            body,
            GlobalTransform {
                matrix: glam::Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0)),
            },
        );
    }
    propagate_subtrees(&mut resources, &[child]);
    assert!(
        (global_x(&resources, child) - 11.0).abs() < 1e-5,
        "the child read {} instead of its parent's 10 plus its own 1",
        global_x(&resources, child),
    );
}

/// An empty list is not a full pass.
#[test]
fn no_roots_publishes_nothing() {
    let (mut resources, body, _, _) = world();
    moved(&mut resources, body, 5.0);
    propagate_subtrees(&mut resources, &[]);
    assert_eq!(global_x(&resources, body), 0.0);
}
