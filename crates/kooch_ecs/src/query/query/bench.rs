//! What a query costs, so #891 stops being an argument.
//!
//! Before the dense columns landed, at 1 000 / 10 000 / 100 000 entities and two components:
//! **18.8 / 19.8 / 29.3 ns per entity**. The rise with count is the `HashMap` losing the cache.

use std::time::Instant;

use crate::archetype_registry::ArchetypeRegistry;
use crate::commands::Commands;
use crate::component::ComponentRegistry;
use crate::component::traits::Component;
use crate::query::Query;
use crate::query::access::AccessTracker;
use kooch_core::resource::Resources;

macro_rules! parts {
    ($($name:ident),+) => {
        $(
            #[derive(Clone, Copy, Default)]
            // Shape and size are the point; nothing reads these back.
            #[allow(dead_code)]
            struct $name {
                x: f32,
                y: f32,
                z: f32,
            }
            impl Component for $name {}
        )+
    };
}

parts!(Position, Velocity, Spin, Scale);

fn resources() -> Resources {
    let mut resources = Resources::new();
    resources.insert(ComponentRegistry::new());
    resources.insert(ArchetypeRegistry::new());
    resources.insert(crate::allocator::EntityAllocator::new());
    resources.insert(AccessTracker::new());
    resources
}

/// `count` entities with two components, plus a third on every `fragment`-th one so the query has
/// more than one archetype to walk.
fn world(count: u32, fragment: u32) -> (Resources, std::time::Duration) {
    let mut resources = resources();
    let mut commands = Commands::new();
    let start = Instant::now();
    for i in 0..count {
        let f = i as f32;
        let mut entity = commands.spawn(&mut resources);
        entity = entity
            .insert(Position { x: f, y: f, z: f })
            .insert(Velocity {
                x: 1.0,
                y: 1.0,
                z: 1.0,
            });
        if fragment > 0 && i % fragment == 0 {
            entity = entity.insert(Spin::default());
        }
        entity.id();
    }
    commands.apply(&mut resources);
    let spawned = start.elapsed();
    (resources, spawned)
}

/// Milliseconds a pass would cost per frame at this count, which is the number that decides whether
/// a scene holds 60 FPS.
fn per_pass(resources: &Resources, count: u32) -> std::time::Duration {
    let mut sum = 0.0f32;
    for _ in 0..3 {
        Query::<(&Position, &Velocity)>::new(resources).for_each(|(p, v)| sum += p.x + v.x);
    }
    let runs = 20;
    let start = Instant::now();
    for _ in 0..runs {
        Query::<(&Position, &Velocity)>::new(resources).for_each(|(p, v)| sum += p.x + v.x);
    }
    std::hint::black_box(sum);
    start.elapsed() / runs
}

#[test]
#[ignore = "a measurement, not an assertion — see #891"]
fn query_cost() {
    for count in [1_000u32, 10_000, 100_000, 1_000_000] {
        let (resources, spawned) = world(count, 0);
        let each = per_pass(&resources, count);
        println!(
            "{count:>9} entities · 2 components · {each:>10.3?} per pass \
             · {:>6.2} ns/entity · spawn {spawned:>10.3?}",
            each.as_nanos() as f64 / count as f64
        );
    }
}

/// The same walk where the entities do **not** all share one archetype, which is every real scene.
#[test]
#[ignore = "a measurement, not an assertion — see #891"]
fn fragmented_cost() {
    for fragment in [2u32, 8, 64] {
        let (resources, _) = world(100_000, fragment);
        let each = per_pass(&resources, 100_000);
        println!(
            "  100000 entities · 2 archetypes (1 in {fragment} has a third component) \
             · {each:>10.3?} per pass · {:>6.2} ns/entity",
            each.as_nanos() as f64 / 100_000.0
        );
    }
}

/// Four components, because the cost per component is what the old map charged twice over.
#[test]
#[ignore = "a measurement, not an assertion — see #891"]
fn four_components() {
    let mut resources = resources();
    let mut commands = Commands::new();
    for i in 0..100_000u32 {
        let f = i as f32;
        commands
            .spawn(&mut resources)
            .insert(Position { x: f, y: f, z: f })
            .insert(Velocity {
                x: 1.0,
                y: 1.0,
                z: 1.0,
            })
            .insert(Spin::default())
            .insert(Scale::default())
            .id();
    }
    commands.apply(&mut resources);

    let mut sum = 0.0f32;
    for _ in 0..3 {
        Query::<(&Position, &Velocity, &Spin, &Scale)>::new(&resources)
            .for_each(|(p, v, s, c)| sum += p.x + v.x + s.x + c.x);
    }
    let runs = 20;
    let start = Instant::now();
    for _ in 0..runs {
        Query::<(&Position, &Velocity, &Spin, &Scale)>::new(&resources)
            .for_each(|(p, v, s, c)| sum += p.x + v.x + s.x + c.x);
    }
    std::hint::black_box(sum);
    let each = start.elapsed() / runs;
    println!(
        "  100000 entities · 4 components · {each:>10.3?} per pass · {:>6.2} ns/entity",
        each.as_nanos() as f64 / 100_000.0
    );
}
