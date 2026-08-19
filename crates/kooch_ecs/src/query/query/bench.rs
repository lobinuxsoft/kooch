//! What a query actually costs today, so #891 stops being an argument.
//!
//! `#[ignore]`d: it is a measurement, not an assertion, and a timing that
//! fails CI on a busy runner teaches nobody anything. Run it with
//! `cargo test -p kooch_ecs --lib query_cost -- --ignored --nocapture`.
//!
//! 🔴 It bounds the ceiling and nothing more. A microbenchmark on a warm
//! cache with one archetype is the **friendliest** case the HashMap will
//! ever see: no archetype churn, no competing memory traffic, every entry
//! recently touched. If the number is small here it cannot be large in a
//! frame; if it is large here, a frame capture is the next step and not
//! the last word.

use std::time::Instant;

use crate::archetype_registry::ArchetypeRegistry;
use crate::commands::Commands;
use crate::component::ComponentRegistry;
use crate::component::traits::Component;
use crate::query::Query;
use crate::query::access::AccessTracker;
use kooch_core::resource::Resources;

#[derive(Clone, Copy)]
struct Position {
    x: f32,
    y: f32,
    z: f32,
}
impl Component for Position {}

#[derive(Clone, Copy)]
struct Velocity {
    x: f32,
    y: f32,
    z: f32,
}
impl Component for Velocity {}

fn world(count: u32) -> Resources {
    let mut resources = Resources::new();
    resources.insert(ComponentRegistry::new());
    resources.insert(ArchetypeRegistry::new());
    resources.insert(kooch_ecs_allocator());
    resources.insert(AccessTracker::new());

    let mut commands = Commands::new();
    for i in 0..count {
        let f = i as f32;
        commands
            .spawn(&mut resources)
            .insert(Position { x: f, y: f, z: f })
            .insert(Velocity {
                x: 1.0,
                y: 1.0,
                z: 1.0,
            });
    }
    commands.apply(&mut resources);
    resources
}

fn kooch_ecs_allocator() -> crate::allocator::EntityAllocator {
    crate::allocator::EntityAllocator::new()
}

#[test]
#[ignore = "a measurement, not an assertion — see #891 stage 0"]
fn query_cost() {
    for count in [1_000u32, 10_000, 100_000] {
        let resources = world(count);

        // Warm the caches, so the number is the friendly case.
        let mut sum = 0.0f32;
        for _ in 0..3 {
            Query::<(&Position, &Velocity)>::new(&resources).for_each(|(p, v)| sum += p.x + v.x);
        }

        let runs = 20;
        let start = Instant::now();
        for _ in 0..runs {
            Query::<(&Position, &Velocity)>::new(&resources).for_each(|(p, v)| sum += p.x + v.x);
        }
        let each = start.elapsed() / runs;

        println!(
            "{count:>7} entities · 2 components · {each:>10.3?} per pass \
             · {:>7.1} ns/entity  (sum {sum})",
            each.as_nanos() as f64 / count as f64
        );
    }
}
