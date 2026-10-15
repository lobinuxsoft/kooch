use super::*;
use crate::spline::Spline;

fn straight(length: f32) -> Spline {
    Spline {
        points: vec![Knot::at(Vec3::ZERO), Knot::at(Vec3::X * length)],
        closed: false,
    }
}

fn placing(mode: u32) -> SplinePlacement {
    SplinePlacement {
        prefab: Some(Guid::new_v4()),
        mode,
        count: 5,
        spacing: 2.0,
        offset: Vec3::ZERO,
    }
}

/// An open curve puts one instance at each end, so `count` instances leave `count - 1` gaps.
#[test]
fn a_count_reaches_both_ends() {
    let poses = poses(&straight(10.0), &placing(SPACING_COUNT));
    assert_eq!(poses.len(), 5);
    assert!(poses[0].position.x.abs() < 1e-3);
    assert!((poses[4].position.x - 10.0).abs() < 1e-3);
}

/// 🔴 The guard that keeps a picked-up collectible gone: only the curve and the numbers move it,
/// so despawning an instance leaves the run alone.
#[test]
fn a_revision_ignores_instances() {
    let spline = straight(10.0);
    let placement = placing(SPACING_COUNT);
    assert_eq!(revision(&spline, &placement), revision(&spline, &placement));
}

/// Moving a knot is what SHOULD rebuild.
#[test]
fn a_moved_knot_changes_it() {
    let placement = placing(SPACING_COUNT);
    assert_ne!(
        revision(&straight(10.0), &placement),
        revision(&straight(11.0), &placement)
    );
}

/// A spacing of zero would divide a curve into infinite instances.
#[test]
fn a_zero_spacing_is_bounded() {
    let mut placement = placing(SPACING_DISTANCE);
    placement.spacing = 0.0;
    assert!(poses(&straight(10.0), &placement).len() <= MAX_INSTANCES);
}

/// 🔴 With nothing configured: the curve's frame is not optional, so banking a knot has to turn
/// the run on its own (#1472).
#[test]
fn a_roll_turns_the_instances() {
    let placement = placing(SPACING_COUNT);

    let flat = straight(10.0);
    let mut banked = straight(10.0);
    for knot in &mut banked.points {
        knot.roll = 45.0;
    }

    let a = poses(&flat, &placement);
    let b = poses(&banked, &placement);
    assert_ne!(a[2].rotation, b[2].rotation, "roll never reached the pose");
    assert_ne!(revision(&flat, &placement), revision(&banked, &placement));
}

/// A dragged tangent grip bends the curve, so the run has to follow it.
#[test]
fn a_broken_tangent_changes_it() {
    let placement = placing(SPACING_COUNT);
    let mut bent = straight(10.0);
    bent.points[0].mode = crate::spline::TANGENT_BROKEN;
    bent.points[0].leaving = Vec3::new(0.0, 4.0, 0.0);
    assert_ne!(
        revision(&straight(10.0), &placement),
        revision(&bent, &placement)
    );
}

/// 🔴 The failure that made the feature look broken (#1481): a prefab that has not loaded yet made
/// `stamp` fail, and recording the revision anyway marked the run as built. The curve never
/// changed again, so it was never retried — one unlucky frame at scene load was permanent.
#[test]
fn an_unplaced_run_is_retried() {
    use crate::component::ComponentRegistry;
    use crate::entity::Entity;
    use kooch_core::resource::Resources;

    let mut resources = Resources::new();
    resources.insert(crate::allocator::EntityAllocator::new());
    resources.insert(ComponentRegistry::new());
    resources.insert(crate::archetype_registry::ArchetypeRegistry::new());
    resources.insert(crate::AccessTracker::new());
    resources.insert(crate::commands::Commands::new());

    let entity = resources
        .get_mut::<crate::allocator::EntityAllocator>()
        .map(|a| a.spawn())
        .unwrap_or(Entity::new(0, 0));

    // A curve with a prefab nothing can resolve — the shape of a scene whose asset is still in
    // flight when the first Update runs.
    if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
        registry.register_cpu_reflected::<Spline>();
        registry.register_cpu_reflected::<SplinePlacement>();
        if let Some(storage) = registry.get_cpu_mut::<Spline>() {
            storage.insert(entity, straight(10.0));
        }
        if let Some(storage) = registry.get_cpu_mut::<SplinePlacement>() {
            storage.insert(entity, placing(SPACING_COUNT));
        }
    }
    // 🔴 The archetype too. A component written only into its own storage is invisible to a
    // `Query`, and a test whose world the system cannot see passes whatever the code does.
    if let Some(archetypes) = resources.get_mut::<crate::archetype_registry::ArchetypeRegistry>() {
        let mut id = archetypes.get_or_create(std::collections::BTreeSet::new());
        for type_id in [
            std::any::TypeId::of::<Spline>(),
            std::any::TypeId::of::<SplinePlacement>(),
        ] {
            id = archetypes.archetype_after_add_dynamic(id, type_id);
        }
        archetypes.register_entity(entity, id);
    }

    place_along_splines(&mut resources);

    let built = resources
        .get::<PlacedRuns>()
        .is_some_and(|runs| runs.runs.contains_key(&entity));
    assert!(
        !built,
        "a run that placed nothing was recorded as built, so it will never be retried"
    );
}
