//! What `gather_entity_data` costs, on the scene the number came from.
//!
//! `Gather · entities` was 4.33 ms of a 13.5 ms frame (#666) on a
//! 610-entity scene, and the HUD reports it as one box. This builds the
//! same shape in-process so the box can be opened without an editor, a
//! GPU, or a person watching a counter.
//!
//! It is `#[ignore]`d: it measures rather than asserts, and a timing
//! assertion on a shared machine is a test that fails for reasons that
//! have nothing to do with the code.
//!
//! ```text
//! cargo test -p kooch_editor_core --lib measure_gather -- --ignored --nocapture
//! ```

use std::collections::HashSet;

use kooch_core::resource::Resources;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::component::{ComponentNames, ComponentRegistry};
use kooch_ecs::dynamic_components::DynamicComponents;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_ecs::name::Name;
use kooch_ecs::persistent_id::{EntityGuid, PersistentId};
use kooch_ecs::query::AccessTracker;
use kooch_ecs::transform::Transform;

use super::super::{gather_entity_data, intern_registry_names};

/// The scene `make_dense_scene` writes: entities in one flat list, each
/// carrying four components. No hierarchy, no colliders — the same
/// omissions the measured scene has.
const ENTITIES: usize = 610;

/// Enough passes that a single scheduler hiccup does not decide the
/// number, few enough that the test stays under a second.
const PASSES: usize = 50;

fn dense_world() -> Resources {
    let mut r = Resources::new();
    let mut alloc = EntityAllocator::new();
    let mut registry = ComponentRegistry::new();
    registry.register_cpu_reflected::<Name>();
    registry.register_cpu_reflected::<Transform>();
    registry.register_cpu_reflected::<GlobalTransform>();
    registry.register_cpu_reflected::<PersistentId>();

    let mut archetypes = ArchetypeRegistry::new();
    let signature = [
        std::any::TypeId::of::<Name>(),
        std::any::TypeId::of::<Transform>(),
        std::any::TypeId::of::<GlobalTransform>(),
        std::any::TypeId::of::<PersistentId>(),
    ]
    .into_iter()
    .collect();
    let archetype = archetypes.get_or_create(signature);

    for index in 0..ENTITIES {
        let entity = alloc.spawn();
        registry
            .get_cpu_mut::<Name>()
            .expect("Name registered")
            .insert(entity, Name::new(format!("Cube {index}")));
        registry
            .get_cpu_mut::<Transform>()
            .expect("Transform registered")
            .insert(entity, Transform::default());
        registry
            .get_cpu_mut::<GlobalTransform>()
            .expect("GlobalTransform registered")
            .insert(entity, GlobalTransform::default());
        registry
            .get_cpu_mut::<PersistentId>()
            .expect("PersistentId registered")
            .insert(
                entity,
                PersistentId::new(EntityGuid::new(index as u64 + 1).expect("nonzero")),
            );
        archetypes.register_entity(entity, archetype);
    }

    r.insert(alloc);
    r.insert(registry);
    r.insert(archetypes);
    r.insert(AccessTracker::new());
    r.insert(DynamicComponents::new());
    r.insert(ComponentNames::new());
    intern_registry_names(&mut r);
    r
}

#[test]
#[ignore = "measures; run with --ignored --nocapture"]
fn what_the_gather_costs() {
    let resources = dense_world();
    let nothing_selected = HashSet::new();

    // One pass first: the allocator's first touch of each size class is
    // not what the frame pays, and warming it keeps that out of the mean.
    let first = gather_entity_data(&resources, &nothing_selected);
    assert_eq!(first.len(), ENTITIES, "every entity gathered");
    let components: usize = first.iter().map(|e| e.components.len()).sum();

    let start = std::time::Instant::now();
    for _ in 0..PASSES {
        let gathered = gather_entity_data(&resources, &nothing_selected);
        std::hint::black_box(&gathered);
    }
    let per_pass = start.elapsed().as_secs_f64() * 1000.0 / PASSES as f64;

    println!(
        "\n  {ENTITIES} entities, {components} components, nothing selected\
         \n  gather_entity_data: {per_pass:.3} ms/pass\n"
    );
}
