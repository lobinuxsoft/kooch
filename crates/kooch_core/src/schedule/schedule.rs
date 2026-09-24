use std::collections::BTreeMap;

use crate::resource::Resources;
use crate::stage::Stage;
use crate::system::{FunctionSystem, GpuSystem, System};

use super::any_system::AnySystem;
use super::catalog::{SystemCatalog, SystemRecord};
use super::gpu_batch::run_gpu_batch;
use super::identity::{SystemInfo, SystemKey, SystemSource};
use super::order::{Order, sort};
use super::toggles::SystemToggles;

/// A system function that operates on resources.
pub type SystemFn = Box<dyn FnMut(&mut Resources) + Send + Sync>;

/// Runs the listed stages in order, each one inside a profiling scope carrying its own name.
macro_rules! run_staged {
    ($self:ident, $resources:ident, $($stage:ident),+ $(,)?) => {
        $({
            profiling::scope!(stringify!($stage));
            $self.run_stage(Stage::$stage, $resources);
        })+
    };
}

/// Organizes systems by stage for ordered execution.
pub struct Schedule {
    stages: BTreeMap<Stage, Vec<AnySystem>>,
    /// Whether startup has already run.
    startup_complete: bool,
    /// Who the next system added belongs to.
    attributing: SystemSource,
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
            attributing: SystemSource::Project,
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
        self.add_ordered(stage, Order::default(), system);
    }

    /// Adds a closure that runs where `order` puts it (#392).
    pub fn add_ordered<F>(&mut self, stage: Stage, order: Order, system: F)
    where
        F: FnMut(&mut Resources) + Send + Sync + 'static,
    {
        let key = self.mint_key(std::any::type_name::<F>());
        let system = AnySystem::cpu(
            Box::new(FunctionSystem::new(system)),
            self.attributing,
            key,
            order,
        );
        self.push(stage, system);
    }

    /// Adds a struct implementing [`System`] at the specified stage.
    pub fn add_cpu_system(&mut self, stage: Stage, system: impl System) {
        self.add_cpu_ordered(stage, Order::default(), system);
    }

    /// Adds a [`System`] that runs where `order` puts it.
    pub fn add_cpu_ordered(&mut self, stage: Stage, order: Order, system: impl System) {
        let key = self.mint_key(system.name());
        let system = AnySystem::cpu(Box::new(system), self.attributing, key, order);
        self.push(stage, system);
    }

    /// Adds a [`GpuSystem`] at the specified stage.
    pub fn add_gpu_system(&mut self, stage: Stage, system: impl GpuSystem) {
        self.add_gpu_ordered(stage, Order::default(), system);
    }

    /// Adds a [`GpuSystem`] that runs where `order` puts it.
    pub fn add_gpu_ordered(&mut self, stage: Stage, order: Order, system: impl GpuSystem) {
        let key = self.mint_key(system.name());
        let system = AnySystem::gpu(Box::new(system), self.attributing, key, order);
        self.push(stage, system);
    }

    /// Files a system under its stage and resolves the stage's order. Sorting here rather than per
    /// frame: registration happens once, `run_stage` happens sixty times a second.
    fn push(&mut self, stage: Stage, system: AnySystem) {
        let systems = self.stages.entry(stage).or_default();
        systems.push(system);
        sort(systems, stage);
    }

    /// Runs all systems in the specified stage.
    pub fn run_stage(&mut self, stage: Stage, resources: &mut Resources) {
        let Some(systems) = self.stages.get_mut(&stage) else {
            return;
        };

        // 🔴 Asked once per stage, not once per system. The common case
        // is that nobody switched anything off, and this is what keeps
        // that case free of a hash lookup per system per frame.
        let any_off = resources
            .get::<SystemToggles>()
            .is_some_and(|toggles| !toggles.is_empty());

        let mut i = 0;
        while i < systems.len() {
            // ⚠️ CPU only. Skipping a GPU system would take it out of the batch `run_gpu_batch`
            // shares an encoder for, which changes how the frame is RECORDED and not just what
            // runs.
            if any_off
                && !systems[i].is_gpu()
                && resources
                    .get::<SystemToggles>()
                    .is_some_and(|toggles| toggles.is_disabled(systems[i].key()))
            {
                i += 1;
                continue;
            }
            if systems[i].is_gpu() {
                // Batch consecutive GPU systems.
                let gpu_start = i;
                while i < systems.len() && systems[i].is_gpu() {
                    i += 1;
                }
                run_gpu_batch(&mut systems[gpu_start..i], resources);
            } else {
                // Its own profiling scope, carrying its own name: a
                // stage is nine systems from five crates, and "one of
                // these nine" is not a thing anyone can act on.
                systems[i].run_cpu(resources);
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
    pub fn run_frame_stages(&mut self, resources: &mut Resources) {
        self.run_pre_physics(resources);
        self.run_post_physics(resources);
    }

    /// Runs the frame stages that precede the fixed timestep loop.
    ///
    /// First → Input → PreUpdate → Update
    pub fn run_pre_physics(&mut self, resources: &mut Resources) {
        run_staged!(self, resources, First, Input, PreUpdate, Update);
    }

    /// Runs the fixed timestep stages once.
    ///
    /// Physics → PostPhysics
    pub fn run_fixed_stages(&mut self, resources: &mut Resources) {
        // 🔴 Here, not in a host's loop: the fixed stages read their own event buffers, and a host
        // that forgot this swap left them reading a list that never moved (#1312).
        crate::event::update_all_fixed_events(resources);
        run_staged!(self, resources, Physics, PostPhysics);
    }

    /// Runs the frame stages that follow the fixed timestep loop.
    pub fn run_post_physics(&mut self, resources: &mut Resources) {
        run_staged!(
            self, resources, PostUpdate, GpuSync, Gpu, PreRender, Render, PostRender, Last,
        );
    }

    #[cfg(test)]
    /// Returns the number of GPU systems in a stage.
    pub fn gpu_system_count(&self, stage: Stage) -> usize {
        self.stages
            .get(&stage)
            .map_or(0, |systems| systems.iter().filter(|s| s.is_gpu()).count())
    }

    /// Returns the names of all systems in a stage, in execution order.
    pub fn system_names(&self, stage: Stage) -> Vec<&str> {
        self.stages.get(&stage).map_or(Vec::new(), |systems| {
            systems.iter().map(|s| s.name()).collect()
        })
    }

    /// Attributes systems added from now on, returning the previous setting so the caller can put
    /// it back.
    pub fn attribute_to(&mut self, source: SystemSource) -> SystemSource {
        std::mem::replace(&mut self.attributing, source)
    }

    /// Every system, in the order a frame runs them.
    pub fn systems(&self) -> Vec<SystemInfo<'_>> {
        RUN_ORDER
            .iter()
            .filter_map(|stage| Some((*stage, self.stages.get(stage)?)))
            .flat_map(|(stage, systems)| {
                systems.iter().map(move |system| SystemInfo {
                    stage,
                    name: system.name(),
                    key: system.key(),
                    source: system.source(),
                    gpu: system.is_gpu(),
                })
            })
            .collect()
    }

    /// The whole schedule, owned, for publishing into `Resources`.
    pub fn catalog(&self) -> SystemCatalog {
        SystemCatalog::new(
            self.systems()
                .into_iter()
                .map(|system| SystemRecord {
                    stage: system.stage,
                    name: system.name.to_owned(),
                    key: system.key.clone(),
                    source: system.source,
                    gpu: system.gpu,
                })
                .collect(),
        )
    }

    /// Builds the key for a system about to be added.
    fn mint_key(&self, name: &str) -> SystemKey {
        let candidate = SystemKey::new(name);
        let nth = self
            .stages
            .values()
            .flatten()
            .filter(|system| system.key().name == candidate.name)
            .count() as u32;
        SystemKey { nth, ..candidate }
    }
}

/// The stages a frame runs, in the order it runs them.
pub const RUN_ORDER: [Stage; 14] = [
    Stage::Startup,
    Stage::First,
    Stage::Input,
    Stage::PreUpdate,
    Stage::Update,
    Stage::Physics,
    Stage::PostPhysics,
    Stage::PostUpdate,
    Stage::GpuSync,
    Stage::Gpu,
    Stage::PreRender,
    Stage::Render,
    Stage::PostRender,
    Stage::Last,
];
