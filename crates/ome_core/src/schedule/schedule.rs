use std::collections::BTreeMap;

use crate::resource::Resources;
use crate::stage::Stage;
use crate::system::{FunctionSystem, GpuSystem, System};

use super::any_system::AnySystem;
use super::gpu_batch::run_gpu_batch;

/// A system function that operates on resources.
///
/// Legacy type alias kept for backward compatibility. Prefer using
/// the [`System`] trait for new code.
pub type SystemFn = Box<dyn FnMut(&mut Resources) + Send + Sync>;

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

    /// Runs a whole frame's non-fixed stages, in order.
    ///
    /// First → Input → PreUpdate → Update → PostUpdate → GpuSync → Gpu →
    /// PreRender → Render → PostRender → Last. For a frame that also
    /// simulates, interleave [`run_fixed_stages`](Self::run_fixed_stages)
    /// between the two halves instead of calling this.
    pub fn run_frame_stages(&mut self, resources: &mut Resources) {
        self.run_pre_physics(resources);
        self.run_post_physics(resources);
    }

    /// Runs the frame stages that precede the fixed timestep loop.
    ///
    /// First → Input → PreUpdate → Update
    pub fn run_pre_physics(&mut self, resources: &mut Resources) {
        for stage in [Stage::First, Stage::Input, Stage::PreUpdate, Stage::Update] {
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

    /// Runs the frame stages that follow the fixed timestep loop.
    ///
    /// PostUpdate → GpuSync → Gpu → PreRender → Render → PostRender → Last
    ///
    /// Transform propagation and the GPU upload live in `PostUpdate` and
    /// `GpuSync`, so they run *after* the solver has written this frame's
    /// poses — the same arrangement Unity, Unreal, Bevy and Godot use.
    /// Running them before the fixed loop would render the previous
    /// frame's simulation.
    pub fn run_post_physics(&mut self, resources: &mut Resources) {
        for stage in [
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
