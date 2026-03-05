//! System traits for CPU and GPU compute systems.
//!
//! - [`System`] — CPU system that processes game logic in Rust.
//! - [`GpuSystem`] — GPU compute system that dispatches work via compute shaders.
//! - [`FunctionSystem`] — wraps a closure into a named [`System`].

use crate::resource::Resources;

/// A CPU system — the core unit of game logic.
///
/// Systems receive mutable access to all [`Resources`] and run sequentially
/// within their stage.
///
/// # Implementing
///
/// ```ignore
/// struct GravitySystem { strength: f32 }
///
/// impl System for GravitySystem {
///     fn run(&mut self, resources: &mut Resources) {
///         // Apply gravity to all entities with Velocity
///     }
///     fn name(&self) -> &str { "GravitySystem" }
/// }
/// ```
///
/// For simple one-off logic, prefer closures via [`App::add_system`]:
///
/// ```ignore
/// app.add_system(Stage::Update, |resources| {
///     // game logic
/// });
/// ```
pub trait System: Send + Sync + 'static {
    /// Executes the system with access to all resources.
    fn run(&mut self, resources: &mut Resources);

    /// Returns the system name for debugging and profiling.
    fn name(&self) -> &str;
}

/// Wraps a closure into a named [`System`].
///
/// Created automatically when adding closures via
/// [`Schedule::add_system`](crate::schedule::Schedule::add_system).
/// The name is derived from the closure's type name.
pub struct FunctionSystem<F> {
    func: F,
    name: &'static str,
}

impl<F> FunctionSystem<F>
where
    F: FnMut(&mut Resources) + Send + Sync + 'static,
{
    /// Wraps a closure, using its type name as the system name.
    pub fn new(func: F) -> Self {
        Self {
            name: std::any::type_name::<F>(),
            func,
        }
    }
}

impl<F> System for FunctionSystem<F>
where
    F: FnMut(&mut Resources) + Send + Sync + 'static,
{
    fn run(&mut self, resources: &mut Resources) {
        (self.func)(resources)
    }

    fn name(&self) -> &str {
        self.name
    }
}

/// A GPU compute system — dispatches parallel work via compute shaders.
///
/// The lifecycle each frame is:
/// 1. **`init`** (once) — create pipeline, bind group layouts.
/// 2. **`prepare`** — update bind groups, write uniforms.
/// 3. **`dispatch`** — record compute pass commands.
///
/// Consecutive GPU systems within a stage are batched into a single
/// command encoder for efficient submission.
///
/// # Note
///
/// During [`prepare`](GpuSystem::prepare), `GpuContext` is temporarily removed
/// from resources to avoid double-borrow. Use the provided `device`/`queue`
/// parameters instead.
///
/// # Example
///
/// ```ignore
/// struct PhysicsIntegrate {
///     pipeline: ComputePipeline,
///     bind_group: Option<BindGroup>,
///     initialized: bool,
/// }
///
/// impl GpuSystem for PhysicsIntegrate {
///     fn init(&mut self, device: &Device, _queue: &Queue) {
///         self.pipeline = create_physics_pipeline(device);
///         self.initialized = true;
///     }
///
///     fn prepare(&mut self, device: &Device, _queue: &Queue, resources: &Resources) {
///         let positions = resources.get::<GpuComponentStorage<Position>>().unwrap();
///         self.bind_group = Some(create_bind_group(device, positions));
///     }
///
///     fn dispatch(&self, pass: &mut ComputePass) {
///         pass.set_pipeline(&self.pipeline);
///         pass.set_bind_group(0, self.bind_group.as_ref().unwrap(), &[]);
///         pass.dispatch_workgroups(256, 1, 1);
///     }
///
///     fn name(&self) -> &str { "PhysicsIntegrate" }
///     fn is_initialized(&self) -> bool { self.initialized }
/// }
/// ```
pub trait GpuSystem: Send + Sync + 'static {
    /// One-time initialization when GPU is first available.
    ///
    /// Create pipelines, bind group layouts, and static resources here.
    fn init(&mut self, device: &wgpu::Device, queue: &wgpu::Queue);

    /// Per-frame preparation before dispatch.
    ///
    /// Update bind groups, write uniforms, and calculate workgroups.
    /// `GpuContext` is NOT in `resources` during this call — use the
    /// provided `device`/`queue`.
    fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, resources: &Resources);

    /// Record compute commands into the pass.
    ///
    /// Set pipeline, bind groups, and dispatch workgroups here.
    fn dispatch(&self, pass: &mut wgpu::ComputePass);

    /// Returns the system name for debugging and profiling.
    fn name(&self) -> &str;

    /// Returns `true` after [`init`](GpuSystem::init) has been called.
    fn is_initialized(&self) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[test]
    fn function_system_runs() {
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let mut sys = FunctionSystem::new(move |_: &mut Resources| {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        });

        let mut resources = Resources::new();
        sys.run(&mut resources);
        sys.run(&mut resources);

        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn function_system_has_name() {
        let sys = FunctionSystem::new(|_: &mut Resources| {});
        assert!(!sys.name().is_empty());
    }

    struct CounterSystem {
        count: u32,
    }

    impl System for CounterSystem {
        fn run(&mut self, _resources: &mut Resources) {
            self.count += 1;
        }

        fn name(&self) -> &str {
            "CounterSystem"
        }
    }

    #[test]
    fn struct_system_runs() {
        let mut sys = CounterSystem { count: 0 };
        let mut resources = Resources::new();

        sys.run(&mut resources);
        sys.run(&mut resources);
        sys.run(&mut resources);

        assert_eq!(sys.count, 3);
    }

    #[test]
    fn struct_system_name() {
        let sys = CounterSystem { count: 0 };
        assert_eq!(sys.name(), "CounterSystem");
    }
}
