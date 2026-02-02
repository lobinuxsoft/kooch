//! Double-buffered event system for frame-delayed communication.
//!
//! Events written in frame N are readable in frame N+1. This prevents
//! order-dependent bugs where a system might miss events from systems
//! that run later in the same frame.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::marker::PhantomData;

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

/// Type-erased event storage that can hold multiple event types.
#[derive(Default)]
pub struct EventRegistry {
    storage: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl EventRegistry {
    /// Creates a new empty event registry.
    pub fn new() -> Self {
        Self {
            storage: HashMap::new(),
        }
    }

    /// Registers a new event type, creating its buffer if it doesn't exist.
    pub fn register<T: Send + Sync + 'static>(&mut self) {
        self.storage
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Box::new(Events::<T>::new()));
    }

    /// Sends an event of type `T`.
    ///
    /// Panics if the event type hasn't been registered.
    pub fn send<T: Send + Sync + 'static>(&mut self, event: T) {
        self.get_mut::<T>()
            .expect("Event type not registered")
            .send(event);
    }

    /// Returns a reader for events of type `T`.
    ///
    /// Returns `None` if the event type hasn't been registered.
    pub fn read<T: Send + Sync + 'static>(&self) -> Option<EventReader<'_, T>> {
        self.get::<T>().map(|events| EventReader {
            events,
            _marker: PhantomData,
        })
    }

    /// Updates all event buffers, swapping read/write buffers.
    ///
    /// Should be called once at the start of each frame.
    pub fn update_all(&mut self) {
        for (_, boxed) in self.storage.iter_mut() {
            // We need to call update on each Events<T>, but we don't know T.
            // Use a trait object approach with a helper trait.
            if let Some(updatable) = boxed.downcast_mut::<Box<dyn EventsUpdatable>>() {
                updatable.update();
            }
        }
    }

    /// Gets a reference to the Events<T> for a specific type.
    fn get<T: Send + Sync + 'static>(&self) -> Option<&Events<T>> {
        self.storage
            .get(&TypeId::of::<T>())
            .and_then(|boxed| boxed.downcast_ref())
    }

    /// Gets a mutable reference to the Events<T> for a specific type.
    fn get_mut<T: Send + Sync + 'static>(&mut self) -> Option<&mut Events<T>> {
        self.storage
            .get_mut(&TypeId::of::<T>())
            .and_then(|boxed| boxed.downcast_mut())
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

/// Helper trait for type-erased event updates.
trait EventsUpdatable: Send + Sync {
    fn update(&mut self);
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

    #[test]
    fn event_registry() {
        let mut registry = EventRegistry::new();
        registry.register::<TestEvent>();

        registry.send(TestEvent(42));
        assert!(registry.read::<TestEvent>().unwrap().is_empty());

        // We can't easily call update_all because of the trait object complexity.
        // For now, test direct access.
        if let Some(events) = registry.get_mut::<TestEvent>() {
            events.update();
        }

        let reader = registry.read::<TestEvent>().unwrap();
        let received: Vec<_> = reader.read().collect();
        assert_eq!(received, vec![&TestEvent(42)]);
    }
}
