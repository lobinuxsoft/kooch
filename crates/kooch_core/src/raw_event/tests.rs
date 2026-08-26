use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Records that it ran, and consumes or not as told.
struct Spy {
    seen: Arc<AtomicUsize>,
    consumes: bool,
}

impl RawEventHandler for Spy {
    fn on_event(&mut self, _window: &dyn Any, _event: &dyn Any) -> bool {
        self.seen.fetch_add(1, Ordering::Relaxed);
        self.consumes
    }
}

fn spy(consumes: bool) -> (Box<dyn RawEventHandler>, Arc<AtomicUsize>) {
    let seen = Arc::new(AtomicUsize::new(0));
    (
        Box::new(Spy {
            seen: Arc::clone(&seen),
            consumes,
        }),
        seen,
    )
}

#[test]
fn an_event_reaches_every_handler_when_nobody_consumes_it() {
    let mut handlers = RawEventHandlers::new();
    let (first, first_seen) = spy(false);
    let (second, second_seen) = spy(false);
    handlers.push(first);
    handlers.push(second);

    assert!(!handlers.dispatch(&(), &()));
    assert_eq!(first_seen.load(Ordering::Relaxed), 1);
    assert_eq!(second_seen.load(Ordering::Relaxed), 1);
}

#[test]
fn a_consumed_event_never_reaches_the_handlers_behind_it() {
    let mut handlers = RawEventHandlers::new();
    let (first, first_seen) = spy(true);
    let (second, second_seen) = spy(false);
    handlers.push(first);
    handlers.push(second);

    assert!(handlers.dispatch(&(), &()));
    assert_eq!(first_seen.load(Ordering::Relaxed), 1);
    assert_eq!(
        second_seen.load(Ordering::Relaxed),
        0,
        "the second handler ran even though the first consumed the event"
    );
}

#[test]
fn registering_a_second_handler_does_not_replace_the_first() {
    let mut handlers = RawEventHandlers::new();
    let (first, first_seen) = spy(false);
    let (second, _) = spy(false);
    handlers.push(first);
    handlers.push(second);

    handlers.dispatch(&(), &());
    assert_eq!(handlers.len(), 2);
    assert_eq!(
        first_seen.load(Ordering::Relaxed),
        1,
        "the handler registered first stopped receiving events"
    );
}
