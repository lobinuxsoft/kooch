use super::*;

#[test]
fn events_double_buffer() {
    let mut events = Events::new();

    // Send events in "frame 1"
    events.send(1);
    events.send(2);

    // Not readable yet (still in write buffer)
    assert!(events.is_empty());

    // Swap buffers (start of "frame 2")
    events.update();

    // Now readable
    let received: Vec<_> = events.read().copied().collect();
    assert_eq!(received, vec![1, 2]);

    // Send new event in "frame 2"
    events.send(3);

    // Frame 2 events not readable, frame 1 events still there
    let received: Vec<_> = events.read().copied().collect();
    assert_eq!(received, vec![1, 2]);

    // Swap buffers (start of "frame 3")
    events.update();

    // Now only frame 2 events readable
    let received: Vec<_> = events.read().copied().collect();
    assert_eq!(received, vec![3]);
}

#[test]
fn events_clear() {
    let mut events = Events::new();
    events.send(1);
    events.update();
    events.send(2);

    events.clear();

    assert!(events.is_empty());
    events.update();
    assert!(events.is_empty());
}

#[derive(Debug, Clone, PartialEq)]
struct TestEvent(i32);

#[derive(Debug)]
struct OtherEvent;

/// The bug this replaced: an event type nobody hardcoded into a runner
/// was never swapped, so it could be sent and never read.
#[test]
fn a_registered_type_is_swapped_by_asking_rather_than_by_name() {
    let mut resources = Resources::new();
    let mut updaters = EventUpdaters::default();
    updaters.register::<TestEvent>();
    resources.insert(updaters);
    resources.insert(Events::<TestEvent>::new());

    resources
        .get_mut::<Events<TestEvent>>()
        .unwrap()
        .send(TestEvent(42));
    assert!(
        resources.get::<Events<TestEvent>>().unwrap().is_empty(),
        "an event should not be readable in the frame it was sent",
    );

    update_all_events(&mut resources);

    let received: Vec<_> = resources
        .get::<Events<TestEvent>>()
        .unwrap()
        .read()
        .cloned()
        .collect();
    assert_eq!(received, vec![TestEvent(42)]);
}

/// `AppExit` really is registered twice — `App::new` and `CorePlugin` —
/// and swapping twice in one frame would discard whatever was written
/// between the swaps.
#[test]
fn registering_a_type_twice_swaps_it_once() {
    let mut updaters = EventUpdaters::default();
    updaters.register::<TestEvent>();
    updaters.register::<TestEvent>();
    assert_eq!(updaters.len(), 1);

    updaters.register::<OtherEvent>();
    assert_eq!(updaters.len(), 2, "a second type should still register");
}

/// A registered type whose buffer was never inserted must not panic:
/// registration and insertion are two calls, and a host may do one.
#[test]
fn a_registered_type_with_no_buffer_is_skipped() {
    let mut resources = Resources::new();
    let mut updaters = EventUpdaters::default();
    updaters.register::<TestEvent>();
    resources.insert(updaters);

    // The assertion is that this returns.
    update_all_events(&mut resources);
}

/// No updaters at all is the state of a hand-built `Resources`, and it
/// has to be silent rather than absent-resource panic.
#[test]
fn no_updaters_is_not_an_error() {
    let mut resources = Resources::new();
    update_all_events(&mut resources);
}
