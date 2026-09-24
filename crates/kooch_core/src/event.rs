//! Double-buffered event system for frame-delayed communication.
//!
//! 🔴 One pair of buffers **per cadence**. A frame runs at whatever the display allows and the
//! fixed stages run at their own rate, so a single pair swapped once a frame threw events away
//! before the fixed stages had a turn: at 360 fps with a 60 Hz step, five collisions in six never
//! reached anybody (#1312). Each cadence swaps its own pair, so every reader gets every event
//! exactly once.

use std::any::TypeId;
use std::marker::PhantomData;

use crate::resource::Resources;

/// One cadence's pair: what is readable, and what is being written for it.
struct Cadence<T> {
    read: Vec<T>,
    write: Vec<T>,
}

impl<T> Cadence<T> {
    fn new() -> Self {
        Self {
            read: Vec::new(),
            write: Vec::new(),
        }
    }

    fn swap(&mut self) {
        std::mem::swap(&mut self.read, &mut self.write);
        self.write.clear();
    }
}

/// Double-buffered storage for events of type `T`, once per cadence.
pub struct Events<T> {
    /// Read by the frame stages, swapped once a frame.
    frame: Cadence<T>,
    /// Read by the fixed stages, swapped once a fixed step.
    fixed: Cadence<T>,
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
            frame: Cadence::new(),
            fixed: Cadence::new(),
        }
    }

    /// Sends an event to every cadence, readable by each on its own next turn.
    pub fn send(&mut self, event: T)
    where
        T: Clone,
    {
        self.frame.write.push(event.clone());
        self.fixed.write.push(event);
    }

    /// The events a frame stage can read: everything sent before this frame's swap.
    pub fn read(&self) -> impl Iterator<Item = &T> {
        self.frame.read.iter()
    }

    /// The events a fixed stage can read: everything sent before this step's swap. A frame is
    /// several steps or none, so this is not the same list as [`read`](Self::read).
    pub fn read_fixed(&self) -> impl Iterator<Item = &T> {
        self.fixed.read.iter()
    }

    /// Swaps the frame pair. Called once per frame at the start of the loop.
    pub fn update(&mut self) {
        self.frame.swap();
    }

    /// Swaps the fixed pair. Called once per fixed step, before the stages that read it.
    pub fn update_fixed(&mut self) {
        self.fixed.swap();
    }

    /// How many events a frame stage can read.
    pub fn len(&self) -> usize {
        self.frame.read.len()
    }

    /// `true` when a frame stage has nothing to read.
    pub fn is_empty(&self) -> bool {
        self.frame.read.is_empty()
    }

    /// Clears every buffer of every cadence.
    pub fn clear(&mut self) {
        self.frame.read.clear();
        self.frame.write.clear();
        self.fixed.read.clear();
        self.fixed.write.clear();
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

/// Swaps every registered event type's **frame** buffers.
pub fn update_all_events(resources: &mut Resources) {
    // Lifted out and put back: each updater takes `&mut Resources`, so the
    // list cannot be borrowed from the same place while they run.
    let Some(updaters) = resources.remove::<EventUpdaters>() else {
        return;
    };
    for (_, swaps) in &updaters.updaters {
        (swaps.frame)(resources);
    }
    resources.insert(updaters);
}

/// Swaps every registered event type's **fixed** buffers.
///
/// Called by [`Schedule::run_fixed_stages`](crate::schedule::Schedule::run_fixed_stages), so no
/// host can forget it and leave its fixed stages reading a list that never moves.
pub fn update_all_fixed_events(resources: &mut Resources) {
    let Some(updaters) = resources.remove::<EventUpdaters>() else {
        return;
    };
    for (_, swaps) in &updaters.updaters {
        (swaps.fixed)(resources);
    }
    resources.insert(updaters);
}

/// The two swaps one event type needs, one per cadence.
#[derive(Clone, Copy)]
struct Swaps {
    frame: fn(&mut Resources),
    fixed: fn(&mut Resources),
}

/// How to swap each registered event type, recorded at registration.
#[derive(Default)]
pub struct EventUpdaters {
    /// Keyed by [`TypeId`] so a type registered twice is swapped once.
    updaters: Vec<(TypeId, Swaps)>,
}

impl EventUpdaters {
    /// Records how to swap `E`, unless it is recorded already.
    pub fn register<E: Send + Sync + 'static>(&mut self) {
        let type_id = TypeId::of::<E>();
        if self.updaters.iter().any(|(known, _)| *known == type_id) {
            return;
        }
        self.updaters.push((
            type_id,
            Swaps {
                frame: swap::<E>,
                fixed: swap_fixed::<E>,
            },
        ));
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

/// The monomorphised frame swap for one event type.
fn swap<E: Send + Sync + 'static>(resources: &mut Resources) {
    if let Some(events) = resources.get_mut::<Events<E>>() {
        events.update();
    }
}

/// The monomorphised fixed swap for one event type.
fn swap_fixed<E: Send + Sync + 'static>(resources: &mut Resources) {
    if let Some(events) = resources.get_mut::<Events<E>>() {
        events.update_fixed();
    }
}

/// Signal sent to request application shutdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppExit;

#[cfg(test)]
mod tests;
