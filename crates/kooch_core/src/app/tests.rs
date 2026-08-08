use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

#[test]
fn new_app() {
    let app = App::new();
    assert!(app.resources.contains::<Events<AppExit>>());
}

#[test]
fn insert_resource() {
    let mut app = App::new();
    app.insert_resource(42_i32);

    assert_eq!(app.resources().get::<i32>(), Some(&42));
}

#[test]
fn add_system() {
    let mut app = App::new();
    let counter = Arc::new(AtomicU32::new(0));
    let counter_clone = counter.clone();

    app.add_system(Stage::Update, move |_| {
        counter_clone.fetch_add(1, Ordering::SeqCst);
    });

    app.schedule.run_stage(Stage::Update, &mut app.resources);

    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

#[derive(Debug, Clone, PartialEq)]
struct TestEvent(i32);

#[test]
fn events() {
    let mut app = App::new();
    app.add_event::<TestEvent>();

    app.send_event(TestEvent(42));

    // Not readable yet (in write buffer)
    let events: Vec<_> = app.read_events::<TestEvent>().unwrap().collect();
    assert!(events.is_empty());

    // Swap buffers
    app.resources
        .get_mut::<Events<TestEvent>>()
        .unwrap()
        .update();

    // Now readable
    let events: Vec<_> = app.read_events::<TestEvent>().unwrap().collect();
    assert_eq!(events, vec![&TestEvent(42)]);
}

struct TestPlugin;

impl Plugin for TestPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(String::from("built"));
    }

    fn finish(&self, app: &mut App) {
        if let Some(s) = app.resources_mut().get_mut::<String>() {
            s.push_str(" and finished");
        }
    }
}

#[test]
fn plugin_lifecycle() {
    let mut app = App::new();
    app.add_plugin(TestPlugin);

    assert_eq!(
        app.resources().get::<String>(),
        Some(&String::from("built"))
    );

    app.finish_plugins();

    assert_eq!(
        app.resources().get::<String>(),
        Some(&String::from("built and finished"))
    );
}
