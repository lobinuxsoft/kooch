//! [`StreamedComponents`] — which components reach a watching editor every frame (#1407).
//!
//! 🔴 While a project plays, the editor asks for **what moved** rather than for the world: pulling
//! the world was 38.9 ms of reflection on 2159 entities and 9.5 ms of a 17.3 ms frame (#1012,
//! #1014). That fast path read exactly one column, `Transform`, so anything else gameplay wrote was
//! invisible in the editor — a vcam's roll arrived and its field of view did not, same frame, same
//! system, same entity.
//!
//! The fix cannot be "stream everything", because that is the 38.9 ms the fast path exists to
//! avoid. So a plugin **declares** the components its systems move at runtime, and only those are
//! diffed. A type nobody declares costs nothing.

use std::any::TypeId;

use crate::component::Component;

/// The components a watching editor is kept up to date on while the project plays.
///
/// Declare one from a plugin's `build` for any component a system **writes during play** and an
/// author has to see. `Transform` is not in here: it has its own cheaper path.
#[derive(Default)]
pub struct StreamedComponents {
    types: Vec<TypeId>,
}

impl StreamedComponents {
    pub fn new() -> Self {
        Self::default()
    }

    /// Declares `T`. Declaring twice is the same as once, so a plugin added by two others is safe.
    pub fn add<T: Component>(&mut self) {
        let id = TypeId::of::<T>();
        if !self.types.contains(&id) {
            self.types.push(id);
        }
    }

    /// Every declared type, in declaration order so a reply is stable frame to frame.
    pub fn iter(&self) -> impl Iterator<Item = &TypeId> {
        self.types.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }
}
