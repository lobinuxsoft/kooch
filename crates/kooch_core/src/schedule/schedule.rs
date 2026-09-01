use std::collections::BTreeMap;

use crate::resource::Resources;
use crate::stage::Stage;
use crate::system::{FunctionSystem, GpuSystem, System};

use super::any_system::AnySystem;
use super::catalog::{SystemCatalog, SystemRecord};
use super::gpu_batch::run_gpu_batch;
use super::identity::{SystemInfo, SystemKey, SystemSource};
use super::toggles::SystemToggles;

/// A system function that operates on resources.
///
/// Legacy type alias kept for backward compatibility. Prefer using
/// the [`System`] trait for new code.
pub type SystemFn = Box<dyn FnMut(&mut Resources) + Send + Sync>;

/// Runs the listed stages in order, each one inside a profiling scope
/// carrying its own name.
///
/// 🔴 The scope has to be expanded per stage instead of written once
/// inside [`Schedule::run_stage`]. `puffin` caches the `ScopeId` in a
/// `static` belonging to the call site and registers it with the *first*
/// name that site ever saw (`profile_scope_custom_if!`), so a single site
/// serving all fourteen stages would report the entire frame under
/// whichever one ran first. Every expansion below is a separate call
/// site, and therefore a separate name.
///
/// Expands to the bare `run_stage` calls when no profiling backend is
/// selected, which is every build that does not ask for one.
macro_rules! run_staged {
    ($self:ident, $resources:ident, $($stage:ident),+ $(,)?) => {
        $({
            profiling::scope!(stringify!($stage));
            $self.run_stage(Stage::$stage, $resources);
        })+
    };
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
    /// Who the next system added belongs to.
    ///
    /// Set around a plugin's `build`, which is what lets every system be
    /// attributed without a word at the call sites. The default is
    /// `Project`: anything added straight onto the `App` outside a
    /// plugin is the game's own `main`.
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
        let key = self.mint_key(std::any::type_name::<F>());
        self.stages.entry(stage).or_default().push(AnySystem::cpu(
            Box::new(FunctionSystem::new(system)),
            self.attributing,
            key,
        ));
    }

    /// Adds a struct implementing [`System`] at the specified stage.
    pub fn add_cpu_system(&mut self, stage: Stage, system: impl System) {
        let key = self.mint_key(system.name());
        self.stages.entry(stage).or_default().push(AnySystem::cpu(
            Box::new(system),
            self.attributing,
            key,
        ));
    }

    /// Adds a [`GpuSystem`] at the specified stage.
    ///
    /// GPU systems are lazily initialized when `GpuContext` first becomes
    /// available. Consecutive GPU systems are batched into one encoder.
    pub fn add_gpu_system(&mut self, stage: Stage, system: impl GpuSystem) {
        let key = self.mint_key(system.name());
        self.stages.entry(stage).or_default().push(AnySystem::gpu(
            Box::new(system),
            self.attributing,
            key,
        ));
    }

    /// Runs all systems in the specified stage.
    ///
    /// CPU systems run inline. Consecutive GPU systems are batched into
    /// a single command encoder and submitted together.
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
            // ⚠️ CPU only. Skipping a GPU system would take it out of the
            // batch `run_gpu_batch` shares an encoder for, which changes
            // how the frame is RECORDED and not just what runs. Moot
            // today — nothing implements `GpuSystem` outside tests — and
            // it belongs with #392, which is what will put real ones
            // there.
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
        run_staged!(self, resources, First, Input, PreUpdate, Update);
    }

    /// Runs the fixed timestep stages once.
    ///
    /// Physics → PostPhysics
    pub fn run_fixed_stages(&mut self, resources: &mut Resources) {
        run_staged!(self, resources, Physics, PostPhysics);
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
        run_staged!(
            self, resources, PostUpdate, GpuSync, Gpu, PreRender, Render, PostRender, Last,
        );
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
        self.stages
            .get(&stage)
            .map_or(0, |systems| systems.iter().filter(|s| !s.is_gpu()).count())
    }

    /// Returns the number of GPU systems in a stage.
    pub fn gpu_system_count(&self, stage: Stage) -> usize {
        self.stages
            .get(&stage)
            .map_or(0, |systems| systems.iter().filter(|s| s.is_gpu()).count())
    }

    /// Returns the total number of systems across all stages.
    pub fn total_system_count(&self) -> usize {
        self.stages.values().map(|v| v.len()).sum()
    }

    /// Returns the names of all systems in a stage, in execution order.
    pub fn system_names(&self, stage: Stage) -> Vec<&str> {
        self.stages.get(&stage).map_or(Vec::new(), |systems| {
            systems.iter().map(|s| s.name()).collect()
        })
    }

    /// Attributes systems added from now on, returning the previous
    /// setting so the caller can put it back.
    ///
    /// `App` wraps a plugin's `build` with this. Restoring rather than
    /// resetting is what lets a plugin add another plugin without the
    /// inner one swallowing the outer one's attribution.
    pub fn attribute_to(&mut self, source: SystemSource) -> SystemSource {
        std::mem::replace(&mut self.attributing, source)
    }

    /// Every system, in the order a frame runs them.
    ///
    /// 🔴 Not `Stage::ALL`, and not the `BTreeMap`'s own order. Both are
    /// declaration order, where `Physics` sits after `Gpu` — but a frame
    /// runs the fixed stages between `Update` and `PostUpdate`. A list
    /// built on either would put physics in a place it never runs.
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
    ///
    /// `systems()` borrows from the schedule, and the schedule lives on
    /// the `App` rather than in `Resources` — so a panel, which is a
    /// system, can only ever see a copy.
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
    ///
    /// `nth` counts the systems already scheduled under the same
    /// canonical name. It stays 0 for anything with a name of its own,
    /// and only climbs for anonymous closures — which share a
    /// `type_name` with every other closure in their module, so without
    /// this a toggle aimed at one would stop all of them.
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

    /// Returns `true` if startup has already completed.
    pub fn is_startup_complete(&self) -> bool {
        self.startup_complete
    }

    /// Resets startup flag (useful for testing).
    pub fn reset_startup(&mut self) {
        self.startup_complete = false;
    }
}

/// The stages a frame runs, in the order it runs them.
///
/// Mirrors `run_startup` + `run_pre_physics` + `run_fixed_stages` +
/// `run_post_physics`. Kept here beside them so the two can be read
/// together, and pinned by a test — a list that drifts from the macros
/// describes a frame nobody runs.
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
