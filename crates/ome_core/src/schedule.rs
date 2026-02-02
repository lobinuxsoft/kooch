//! System scheduling and execution.
//!
//! The schedule organizes systems by stage and executes them in order.
//! Systems are functions that receive mutable access to resources.

use std::collections::BTreeMap;

use crate::resource::Resources;
use crate::stage::Stage;

/// A system function that operates on resources.
///
/// Systems are the core unit of logic in the engine. Each system receives
/// mutable access to the resource storage and can read/write any resource.
///
/// # Example
/// ```ignore
/// fn my_system(resources: &mut Resources) {
///     if let Some(time) = resources.get::<Time>() {
///         println!("Delta: {:?}", time.delta());
///     }
/// }
/// ```
pub type SystemFn = Box<dyn FnMut(&mut Resources) + Send + Sync>;

/// Organizes systems by stage for ordered execution.
///
/// Systems are stored in a `BTreeMap` keyed by `Stage`, ensuring they
/// execute in the correct stage order. Within a stage, systems run in
/// the order they were added.
pub struct Schedule {
    stages: BTreeMap<Stage, Vec<SystemFn>>,
    /// Whether startup has already run.
    startup_complete: bool,
}

impl Default for Schedule {
    fn default() -> Self {
        Self::new()
    }
}

impl Schedule {
    /// Creates a new empty schedule.
    pub fn new() -> Self {
        Self {
            stages: BTreeMap::new(),
            startup_complete: false,
        }
    }

    /// Adds a system to run at the specified stage.
    ///
    /// Systems within the same stage run in the order they were added.
    ///
    /// # Example
    /// ```ignore
    /// schedule.add_system(Stage::Update, |resources| {
    ///     // Game logic here
    /// });
    /// ```
    pub fn add_system<F>(&mut self, stage: Stage, system: F)
    where
        F: FnMut(&mut Resources) + Send + Sync + 'static,
    {
        self.stages
            .entry(stage)
            .or_insert_with(Vec::new)
            .push(Box::new(system));
    }

    /// Runs all systems in the specified stage.
    pub fn run_stage(&mut self, stage: Stage, resources: &mut Resources) {
        if let Some(systems) = self.stages.get_mut(&stage) {
            for system in systems.iter_mut() {
                system(resources);
            }
        }
    }

    /// Runs the startup stage if it hasn't run yet.
    ///
    /// Returns `true` if startup was run, `false` if already complete.
    pub fn run_startup(&mut self, resources: &mut Resources) -> bool {
        if self.startup_complete {
            return false;
        }
        self.run_stage(Stage::Startup, resources);
        self.startup_complete = true;
        true
    }

    /// Runs all non-fixed stages in order (excluding Startup).
    ///
    /// This runs: First → Input → PreUpdate → Update → PostUpdate →
    /// GpuSync → Gpu → PreRender → Render → PostRender → Last
    pub fn run_frame_stages(&mut self, resources: &mut Resources) {
        for stage in [
            Stage::First,
            Stage::Input,
            Stage::PreUpdate,
            Stage::Update,
            Stage::PostUpdate,
            Stage::GpuSync,
            Stage::Gpu,
            Stage::PreRender,
            Stage::Render,
            Stage::PostRender,
            Stage::Last,
        ] {
            self.run_stage(stage, resources);
        }
    }

    /// Runs the pre-physics frame stages.
    ///
    /// First → Input → PreUpdate → Update → PostUpdate → GpuSync → Gpu
    pub fn run_pre_physics(&mut self, resources: &mut Resources) {
        for stage in [
            Stage::First,
            Stage::Input,
            Stage::PreUpdate,
            Stage::Update,
            Stage::PostUpdate,
            Stage::GpuSync,
            Stage::Gpu,
        ] {
            self.run_stage(stage, resources);
        }
    }

    /// Runs the fixed timestep stages once.
    ///
    /// Physics → PostPhysics
    pub fn run_fixed_stages(&mut self, resources: &mut Resources) {
        self.run_stage(Stage::Physics, resources);
        self.run_stage(Stage::PostPhysics, resources);
    }

    /// Runs the post-physics frame stages.
    ///
    /// PreRender → Render → PostRender → Last
    pub fn run_post_physics(&mut self, resources: &mut Resources) {
        for stage in [
            Stage::PreRender,
            Stage::Render,
            Stage::PostRender,
            Stage::Last,
        ] {
            self.run_stage(stage, resources);
        }
    }

    /// Returns `true` if any systems are registered for the stage.
    pub fn has_systems(&self, stage: Stage) -> bool {
        self.stages
            .get(&stage)
            .map_or(false, |systems| !systems.is_empty())
    }

    /// Returns the number of systems registered for a stage.
    pub fn system_count(&self, stage: Stage) -> usize {
        self.stages.get(&stage).map_or(0, |systems| systems.len())
    }

    /// Returns the total number of systems across all stages.
    pub fn total_system_count(&self) -> usize {
        self.stages.values().map(|v| v.len()).sum()
    }

    /// Returns `true` if startup has already completed.
    pub fn is_startup_complete(&self) -> bool {
        self.startup_complete
    }

    /// Resets startup flag (useful for testing).
    pub fn reset_startup(&mut self) {
        self.startup_complete = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[test]
    fn add_and_run_system() {
        let mut schedule = Schedule::new();
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        schedule.add_system(Stage::Update, move |_resources| {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        });

        let mut resources = Resources::new();
        schedule.run_stage(Stage::Update, &mut resources);

        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn systems_run_in_order() {
        let mut schedule = Schedule::new();
        let order = Arc::new(std::sync::Mutex::new(Vec::new()));

        let order1 = order.clone();
        schedule.add_system(Stage::Update, move |_| {
            order1.lock().unwrap().push(1);
        });

        let order2 = order.clone();
        schedule.add_system(Stage::Update, move |_| {
            order2.lock().unwrap().push(2);
        });

        let order3 = order.clone();
        schedule.add_system(Stage::Update, move |_| {
            order3.lock().unwrap().push(3);
        });

        let mut resources = Resources::new();
        schedule.run_stage(Stage::Update, &mut resources);

        assert_eq!(*order.lock().unwrap(), vec![1, 2, 3]);
    }

    #[test]
    fn stages_run_in_order() {
        let mut schedule = Schedule::new();
        let order = Arc::new(std::sync::Mutex::new(Vec::new()));

        let order1 = order.clone();
        schedule.add_system(Stage::First, move |_| {
            order1.lock().unwrap().push("First");
        });

        let order2 = order.clone();
        schedule.add_system(Stage::Update, move |_| {
            order2.lock().unwrap().push("Update");
        });

        let order3 = order.clone();
        schedule.add_system(Stage::Last, move |_| {
            order3.lock().unwrap().push("Last");
        });

        let mut resources = Resources::new();
        schedule.run_frame_stages(&mut resources);

        let recorded = order.lock().unwrap();
        assert_eq!(*recorded, vec!["First", "Update", "Last"]);
    }

    #[test]
    fn startup_runs_once() {
        let mut schedule = Schedule::new();
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        schedule.add_system(Stage::Startup, move |_| {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        });

        let mut resources = Resources::new();

        assert!(schedule.run_startup(&mut resources));
        assert!(!schedule.run_startup(&mut resources));
        assert!(!schedule.run_startup(&mut resources));

        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn system_counts() {
        let mut schedule = Schedule::new();

        schedule.add_system(Stage::Update, |_| {});
        schedule.add_system(Stage::Update, |_| {});
        schedule.add_system(Stage::Render, |_| {});

        assert_eq!(schedule.system_count(Stage::Update), 2);
        assert_eq!(schedule.system_count(Stage::Render), 1);
        assert_eq!(schedule.system_count(Stage::Physics), 0);
        assert_eq!(schedule.total_system_count(), 3);

        assert!(schedule.has_systems(Stage::Update));
        assert!(!schedule.has_systems(Stage::Physics));
    }
}
