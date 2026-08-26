use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

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
