//! Test code for `order`, in its own file.

use crate::resource::Resources;
use crate::schedule::{Order, Schedule};
use crate::stage::Stage;

/// Pushes its name onto the log in `Resources`.
fn logger(name: &'static str) -> impl FnMut(&mut Resources) + Send + Sync + 'static {
    move |resources: &mut Resources| {
        if let Some(log) = resources.get_mut::<Vec<&'static str>>() {
            log.push(name);
        }
    }
}

fn ran(schedule: &mut Schedule) -> Vec<&'static str> {
    let mut resources = Resources::new();
    resources.insert(Vec::<&'static str>::new());
    schedule.run_stage(Stage::Update, &mut resources);
    resources.get::<Vec<&'static str>>().cloned().unwrap()
}

/// The name is the handle: `late` registers first and still runs last.
#[test]
fn an_after_runs_later() {
    let mut schedule = Schedule::new();
    schedule.add_cpu_ordered(Stage::Update, Order::after("early"), Late);
    schedule.add_cpu_system(Stage::Update, Early);
    assert_eq!(ran(&mut schedule), ["early", "late"]);
}

#[test]
fn a_before_runs_earlier() {
    let mut schedule = Schedule::new();
    schedule.add_cpu_system(Stage::Update, Early);
    schedule.add_cpu_ordered(Stage::Update, Order::before("early"), Late);
    assert_eq!(ran(&mut schedule), ["late", "early"]);
}

/// Two systems with nothing to say about each other keep the order they were added in.
#[test]
fn registration_breaks_a_tie() {
    let mut schedule = Schedule::new();
    schedule.add_cpu_system(Stage::Update, Early);
    schedule.add_cpu_system(Stage::Update, Late);
    schedule.add_cpu_ordered(Stage::Update, Order::after("early"), Third);
    assert_eq!(ran(&mut schedule), ["early", "late", "third"]);
}

/// A constraint against a system nobody registered is dropped, not an error: the plugin that owns
/// the name may not be loaded.
#[test]
fn an_unknown_name_is_dropped() {
    let mut schedule = Schedule::new();
    schedule.add_cpu_ordered(Stage::Update, Order::after("absent"), Early);
    assert_eq!(ran(&mut schedule), ["early"]);
}

/// A contradiction runs as registered rather than dropping a system.
#[test]
fn a_cycle_keeps_every_system() {
    let mut schedule = Schedule::new();
    schedule.add_cpu_ordered(Stage::Update, Order::after("late"), Early);
    schedule.add_cpu_ordered(Stage::Update, Order::after("early"), Late);
    assert_eq!(ran(&mut schedule), ["early", "late"]);
}

/// What `systems()` reports is the order the frame runs, constraints resolved.
#[test]
fn the_catalog_shows_the_order() {
    let mut schedule = Schedule::new();
    schedule.add_cpu_ordered(Stage::Update, Order::after("early"), Late);
    schedule.add_cpu_system(Stage::Update, Early);
    let systems = schedule.systems();
    let names: Vec<&str> = systems.iter().map(|s| s.short_name()).collect();
    assert_eq!(names, ["early", "late"]);
}

macro_rules! named_system {
    ($type:ident, $name:literal) => {
        struct $type;
        impl crate::system::System for $type {
            fn run(&mut self, resources: &mut Resources) {
                logger($name)(resources);
            }
            fn name(&self) -> &str {
                $name
            }
        }
    };
}

named_system!(Early, "early");
named_system!(Late, "late");
named_system!(Third, "third");
