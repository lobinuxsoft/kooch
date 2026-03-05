//! System scheduling and execution.
//!
//! The schedule organizes systems by stage and executes them in order.
//! Both CPU [`System`]s and GPU [`GpuSystem`]s are supported, with
//! consecutive GPU systems batched into a single command encoder.

use std::collections::BTreeMap;

use crate::resource::Resources;
use crate::stage::Stage;
use crate::system::{FunctionSystem, GpuSystem, System};

/// A system function that operates on resources.
///
/// Legacy type alias kept for backward compatibility. Prefer using
/// the [`System`] trait for new code.
pub type SystemFn = Box<dyn FnMut(&mut Resources) + Send + Sync>;

/// Either a CPU or GPU system.
enum AnySystem {
    Cpu(Box<dyn System>),
    Gpu(Box<dyn GpuSystem>),
}

impl AnySystem {
    fn name(&self) -> &str {
        match self {
            Self::Cpu(s) => s.name(),
            Self::Gpu(s) => s.name(),
        }
    }

    fn is_gpu(&self) -> bool {
        matches!(self, Self::Gpu(_))
    }
}

/// Organizes systems by stage for ordered execution.
///
/// Systems are stored in a `BTreeMap` keyed by `Stage`, ensuring they
/// execute in the correct stage order. Within a stage, systems run in
/// the order they were added.
///
/// Consecutive GPU systems within a stage are batched into a single
/// command encoder submission for efficiency.
pub struct Schedule {
    stages: BTreeMap<Stage, Vec<AnySystem>>,
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

    /// Adds a closure as a CPU system at the specified stage.
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
            .or_default()
            .push(AnySystem::Cpu(Box::new(FunctionSystem::new(system))));
    }

    /// Adds a struct implementing [`System`] at the specified stage.
    pub fn add_cpu_system(&mut self, stage: Stage, system: impl System) {
        self.stages
            .entry(stage)
            .or_default()
            .push(AnySystem::Cpu(Box::new(system)));
    }

    /// Adds a [`GpuSystem`] at the specified stage.
    ///
    /// GPU systems are lazily initialized when `GpuContext` first becomes
    /// available. Consecutive GPU systems are batched into one encoder.
    pub fn add_gpu_system(&mut self, stage: Stage, system: impl GpuSystem) {
        self.stages
            .entry(stage)
            .or_default()
            .push(AnySystem::Gpu(Box::new(system)));
    }

    /// Runs all systems in the specified stage.
    ///
    /// CPU systems run inline. Consecutive GPU systems are batched into
    /// a single command encoder and submitted together.
    pub fn run_stage(&mut self, stage: Stage, resources: &mut Resources) {
        let Some(systems) = self.stages.get_mut(&stage) else {
            return;
        };

        let mut i = 0;
        while i < systems.len() {
            if systems[i].is_gpu() {
                // Batch consecutive GPU systems.
                let gpu_start = i;
                while i < systems.len() && systems[i].is_gpu() {
                    i += 1;
                }
                run_gpu_batch(&mut systems[gpu_start..i], resources);
            } else {
                if let AnySystem::Cpu(sys) = &mut systems[i] {
                    sys.run(resources);
                }
                i += 1;
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

    /// Returns the number of CPU systems in a stage.
    pub fn cpu_system_count(&self, stage: Stage) -> usize {
        self.stages.get(&stage).map_or(0, |systems| {
            systems.iter().filter(|s| !s.is_gpu()).count()
        })
    }

    /// Returns the number of GPU systems in a stage.
    pub fn gpu_system_count(&self, stage: Stage) -> usize {
        self.stages.get(&stage).map_or(0, |systems| {
            systems.iter().filter(|s| s.is_gpu()).count()
        })
    }

    /// Returns the total number of systems across all stages.
    pub fn total_system_count(&self) -> usize {
        self.stages.values().map(|v| v.len()).sum()
    }

    /// Returns the names of all systems in a stage, in execution order.
    pub fn system_names(&self, stage: Stage) -> Vec<&str> {
        self.stages
            .get(&stage)
            .map_or(Vec::new(), |systems| systems.iter().map(|s| s.name()).collect())
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

/// Runs a batch of consecutive GPU systems with one encoder submission.
fn run_gpu_batch(systems: &mut [AnySystem], resources: &mut Resources) {
    use crate::gpu::GpuContext;

    let Some(gpu) = resources.remove::<GpuContext>() else {
        let names: Vec<&str> = systems.iter().map(|s| s.name()).collect();
        tracing::warn!(
            systems = ?names,
            "GpuContext not available, skipping GPU systems",
        );
        return;
    };

    // Init + prepare phase (GpuContext removed from resources).
    for sys in systems.iter_mut() {
        if let AnySystem::Gpu(gpu_sys) = sys {
            if !gpu_sys.is_initialized() {
                gpu_sys.init(gpu.device(), gpu.queue());
                tracing::debug!(system = gpu_sys.name(), "GPU system initialized");
            }
            gpu_sys.prepare(gpu.device(), gpu.queue(), resources);
        }
    }

    // Dispatch phase — one encoder, one pass per system.
    let mut encoder =
        gpu.device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("gpu_system_encoder"),
            });

    for sys in systems.iter() {
        if let AnySystem::Gpu(gpu_sys) = sys {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(gpu_sys.name()),
                timestamp_writes: None,
            });
            gpu_sys.dispatch(&mut pass);
        }
    }

    gpu.queue().submit(std::iter::once(encoder.finish()));

    // Restore GpuContext.
    resources.insert(gpu);
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

        schedule.add_system(Stage::Update, move |_resources: &mut Resources| {
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
        schedule.add_system(Stage::Update, move |_: &mut Resources| {
            order1.lock().unwrap().push(1);
        });

        let order2 = order.clone();
        schedule.add_system(Stage::Update, move |_: &mut Resources| {
            order2.lock().unwrap().push(2);
        });

        let order3 = order.clone();
        schedule.add_system(Stage::Update, move |_: &mut Resources| {
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
        schedule.add_system(Stage::First, move |_: &mut Resources| {
            order1.lock().unwrap().push("First");
        });

        let order2 = order.clone();
        schedule.add_system(Stage::Update, move |_: &mut Resources| {
            order2.lock().unwrap().push("Update");
        });

        let order3 = order.clone();
        schedule.add_system(Stage::Last, move |_: &mut Resources| {
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

        schedule.add_system(Stage::Startup, move |_: &mut Resources| {
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

        schedule.add_system(Stage::Update, |_: &mut Resources| {});
        schedule.add_system(Stage::Update, |_: &mut Resources| {});
        schedule.add_system(Stage::Render, |_: &mut Resources| {});

        assert_eq!(schedule.system_count(Stage::Update), 2);
        assert_eq!(schedule.system_count(Stage::Render), 1);
        assert_eq!(schedule.system_count(Stage::Physics), 0);
        assert_eq!(schedule.total_system_count(), 3);

        assert!(schedule.has_systems(Stage::Update));
        assert!(!schedule.has_systems(Stage::Physics));
    }

    // --- New tests for System trait and GPU systems ---

    struct IncrementSystem {
        amount: u32,
    }

    impl System for IncrementSystem {
        fn run(&mut self, resources: &mut Resources) {
            if let Some(counter) = resources.get_mut::<Arc<AtomicU32>>() {
                counter.fetch_add(self.amount, Ordering::SeqCst);
            }
        }

        fn name(&self) -> &str {
            "IncrementSystem"
        }
    }

    #[test]
    fn add_cpu_system_struct() {
        let mut schedule = Schedule::new();
        let counter = Arc::new(AtomicU32::new(0));

        schedule.add_cpu_system(Stage::Update, IncrementSystem { amount: 5 });

        let mut resources = Resources::new();
        resources.insert(counter.clone());

        schedule.run_stage(Stage::Update, &mut resources);
        assert_eq!(counter.load(Ordering::SeqCst), 5);
    }

    #[test]
    fn mixed_closure_and_struct_systems() {
        let mut schedule = Schedule::new();
        let order = Arc::new(std::sync::Mutex::new(Vec::new()));

        let order1 = order.clone();
        schedule.add_system(Stage::Update, move |_: &mut Resources| {
            order1.lock().unwrap().push("closure");
        });

        schedule.add_cpu_system(Stage::Update, NamedSystem("struct"));

        let order3 = order.clone();
        schedule.add_system(Stage::Update, move |_: &mut Resources| {
            order3.lock().unwrap().push("closure2");
        });

        let mut resources = Resources::new();
        resources.insert(order.clone());
        schedule.run_stage(Stage::Update, &mut resources);

        assert_eq!(*order.lock().unwrap(), vec!["closure", "struct", "closure2"]);
    }

    struct NamedSystem(&'static str);

    impl System for NamedSystem {
        fn run(&mut self, resources: &mut Resources) {
            if let Some(order) = resources.get_mut::<Arc<std::sync::Mutex<Vec<&'static str>>>>() {
                order.lock().unwrap().push(self.0);
            }
        }

        fn name(&self) -> &str {
            self.0
        }
    }

    #[test]
    fn system_names_reported() {
        let mut schedule = Schedule::new();

        schedule.add_cpu_system(Stage::Update, NamedSystem("Alpha"));
        schedule.add_cpu_system(Stage::Update, NamedSystem("Beta"));

        let names = schedule.system_names(Stage::Update);
        assert_eq!(names, vec!["Alpha", "Beta"]);
    }

    #[test]
    fn cpu_gpu_system_counts() {
        let mut schedule = Schedule::new();

        schedule.add_system(Stage::Update, |_: &mut Resources| {});
        schedule.add_cpu_system(Stage::Update, NamedSystem("A"));

        assert_eq!(schedule.cpu_system_count(Stage::Update), 2);
        assert_eq!(schedule.gpu_system_count(Stage::Update), 0);
        assert_eq!(schedule.system_count(Stage::Update), 2);
    }

    #[test]
    fn gpu_systems_skipped_without_gpu_context() {
        // GPU systems should be silently skipped when no GpuContext is available.
        let mut schedule = Schedule::new();

        struct DummyGpu;
        impl GpuSystem for DummyGpu {
            fn init(&mut self, _: &wgpu::Device, _: &wgpu::Queue) {}
            fn prepare(&mut self, _: &wgpu::Device, _: &wgpu::Queue, _: &Resources) {}
            fn dispatch(&self, _: &mut wgpu::ComputePass) {}
            fn name(&self) -> &str { "DummyGpu" }
            fn is_initialized(&self) -> bool { true }
        }

        schedule.add_gpu_system(Stage::Physics, DummyGpu);

        assert_eq!(schedule.gpu_system_count(Stage::Physics), 1);

        // Should not panic even without GpuContext.
        let mut resources = Resources::new();
        schedule.run_stage(Stage::Physics, &mut resources);
    }

    #[test]
    fn cpu_systems_still_run_when_gpu_systems_skipped() {
        let mut schedule = Schedule::new();
        let counter = Arc::new(AtomicU32::new(0));

        struct DummyGpu;
        impl GpuSystem for DummyGpu {
            fn init(&mut self, _: &wgpu::Device, _: &wgpu::Queue) {}
            fn prepare(&mut self, _: &wgpu::Device, _: &wgpu::Queue, _: &Resources) {}
            fn dispatch(&self, _: &mut wgpu::ComputePass) {}
            fn name(&self) -> &str { "DummyGpu" }
            fn is_initialized(&self) -> bool { true }
        }

        let counter_clone = counter.clone();
        schedule.add_system(Stage::Update, move |_: &mut Resources| {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        });
        schedule.add_gpu_system(Stage::Update, DummyGpu);
        let counter_clone = counter.clone();
        schedule.add_system(Stage::Update, move |_: &mut Resources| {
            counter_clone.fetch_add(10, Ordering::SeqCst);
        });

        let mut resources = Resources::new();
        schedule.run_stage(Stage::Update, &mut resources);

        // Both CPU systems should have run, GPU skipped.
        assert_eq!(counter.load(Ordering::SeqCst), 11);
    }
}
