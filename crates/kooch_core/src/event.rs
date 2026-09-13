//! Double-buffered event system for frame-delayed communication.

use std::any::TypeId;
use std::marker::PhantomData;

use crate::resource::Resources;

/// Double-buffered storage for events of type `T`.
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
#[derive(Default)]
pub struct EventUpdaters {
    /// Keyed by [`TypeId`] so a type registered twice is swapped once.
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
mod tests;
