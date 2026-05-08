use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use crate::resource::Resources;
use crate::stage::Stage;
use crate::system::{GpuSystem, System};

use super::Schedule;

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
