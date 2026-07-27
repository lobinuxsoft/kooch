//! Double-buffered event system for frame-delayed communication.
//!
//! Events written in frame N are readable in frame N+1. This prevents
//! order-dependent bugs where a system might miss events from systems
//! that run later in the same frame.

use std::any::TypeId;
use std::marker::PhantomData;

use crate::resource::Resources;

/// Double-buffered storage for events of type `T`.
///
/// Events are written to a "write" buffer during a frame, then during
/// the update phase, buffers are swapped so readers see last frame's events.
pub struct Events<T> {
    /// Events from the previous frame (readable).
    read_buffer: Vec<T>,
    /// Events being written this frame.
    write_buffer: Vec<T>,
}

impl<T> Default for Events<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Events<T> {
    /// Creates a new empty event buffer.
    pub fn new() -> Self {
        Self {
            read_buffer: Vec::new(),
            write_buffer: Vec::new(),
        }
    }

    /// Sends an event, adding it to the write buffer.
    ///
    /// The event will be readable starting next frame.
    pub fn send(&mut self, event: T) {
        self.write_buffer.push(event);
    }

    /// Returns an iterator over events from the previous frame.
    pub fn read(&self) -> impl Iterator<Item = &T> {
        self.read_buffer.iter()
    }

    /// Swaps buffers and clears the old read buffer.
    ///
    /// Called once per frame at the start of the game loop.
    pub fn update(&mut self) {
        std::mem::swap(&mut self.read_buffer, &mut self.write_buffer);
        self.write_buffer.clear();
    }

    /// Returns the number of readable events (from previous frame).
    pub fn len(&self) -> usize {
        self.read_buffer.len()
    }

    /// Returns `true` if there are no readable events.
    pub fn is_empty(&self) -> bool {
        self.read_buffer.is_empty()
    }

    /// Clears both buffers.
    pub fn clear(&mut self) {
        self.read_buffer.clear();
        self.write_buffer.clear();
    }
}

/// A reader that provides access to events from the previous frame.
pub struct EventReader<'a, T> {
    events: &'a Events<T>,
    _marker: PhantomData<T>,
}

impl<'a, T> EventReader<'a, T> {
    /// Returns an iterator over the events.
    pub fn read(&self) -> impl Iterator<Item = &T> {
        self.events.read()
    }

    /// Returns the number of events.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Returns `true` if there are no events.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

/// Swaps every registered event type's buffers.
///
/// # Why this is not a hardcoded list
///
/// It used to be — twice. The default runner swapped `AppExit` and nothing
/// else; the winit runner swapped three types under a comment claiming it
/// handled "all registered event types". Every event any plugin added was
/// invisible to both, so `send` wrote into a buffer that never became
/// readable and `read` returned empty forever. Collision events (#561) were
/// the first feature to notice, four months on.
///
/// So registration records how to swap, and a runner asks rather than
/// remembering. A new event type is delivered because
/// [`add_event`](crate::app::App::add_event) was called, not because
/// someone also edited two runners.
pub fn update_all_events(resources: &mut Resources) {
    // Lifted out and put back: each updater takes `&mut Resources`, so the
    // list cannot be borrowed from the same place while they run.
    let Some(updaters) = resources.remove::<EventUpdaters>() else {
        return;
    };
    for (_, updater) in &updaters.updaters {
        updater(resources);
    }
    resources.insert(updaters);
}

/// How to swap each registered event type, recorded at registration.
///
/// A plain function pointer per type: `add_event` is generic, so the
/// compiler monomorphises one swap per event type and the list needs no
/// type erasure of its own — which is what the deleted `EventRegistry`
/// attempted and never managed. Its `update_all` downcast to a trait object
/// that nothing implemented, and its own test said so in a comment.
#[derive(Default)]
pub struct EventUpdaters {
    /// Keyed by [`TypeId`] so a type registered twice is swapped once.
    ///
    /// Not hypothetical: `AppExit` is registered by both `App::new` and
    /// `CorePlugin`, and swapping twice in a frame would discard whatever
    /// was written between the two swaps.
    updaters: Vec<(TypeId, fn(&mut Resources))>,
}

impl EventUpdaters {
    /// Records how to swap `E`, unless it is recorded already.
    pub fn register<E: Send + Sync + 'static>(&mut self) {
        let type_id = TypeId::of::<E>();
        if self.updaters.iter().any(|(known, _)| *known == type_id) {
            return;
        }
        self.updaters.push((type_id, swap::<E>));
    }

    /// How many event types will be swapped.
    pub fn len(&self) -> usize {
        self.updaters.len()
    }

    /// `true` when nothing is registered.
    pub fn is_empty(&self) -> bool {
        self.updaters.is_empty()
    }
}

/// The monomorphised swap for one event type.
fn swap<E: Send + Sync + 'static>(resources: &mut Resources) {
    if let Some(events) = resources.get_mut::<Events<E>>() {
        events.update();
    }
}

/// Signal sent to request application shutdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppExit;

#[cfg(test)]
mod tests {
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
}
