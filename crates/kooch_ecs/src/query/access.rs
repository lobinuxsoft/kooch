//! Runtime access tracking for component queries.
//!
//! [`AccessTracker`] provides RefCell-like borrow checking at the storage
//! level, allowing multiple queries to coexist safely as long as they don't
//! create conflicting mutable borrows on the same component type.

use std::any::TypeId;
use std::collections::HashMap;
use std::sync::atomic::{AtomicIsize, Ordering};

/// Tracks active borrows on component storages at runtime.
///
/// Similar to `RefCell` but per-TypeId: allows multiple shared borrows OR
/// one exclusive borrow per component type. Uses `RwLock` for auto-registration
/// of new component types on first access.
pub struct AccessTracker {
    borrows: std::sync::RwLock<HashMap<TypeId, AtomicIsize>>,
}

impl AccessTracker {
    pub fn new() -> Self {
        Self {
            borrows: std::sync::RwLock::new(HashMap::new()),
        }
    }

    /// Ensures a TypeId entry exists, creating it if needed.
    fn ensure_registered(&self, type_id: TypeId) {
        {
            let borrows = self.borrows.read().unwrap();
            if borrows.contains_key(&type_id) {
                return;
            }
        }
        let mut borrows = self.borrows.write().unwrap();
        borrows
            .entry(type_id)
            .or_insert_with(|| AtomicIsize::new(0));
    }

    /// Acquires a shared (read) borrow on a component type.
    ///
    /// # Panics
    ///
    /// Panics if there is an active mutable borrow on the same type.
    pub fn borrow_read(&self, type_id: TypeId) {
        self.ensure_registered(type_id);
        let borrows = self.borrows.read().unwrap();
        let atomic = borrows.get(&type_id).unwrap();
        let current = atomic.load(Ordering::Acquire);
        assert!(
            current >= 0,
            "cannot borrow component as immutable: already borrowed as mutable"
        );
        atomic.store(current + 1, Ordering::Release);
    }

    /// Releases a shared (read) borrow on a component type.
    ///
    /// Returns `false` if there was no active read borrow (type not registered
    /// or not borrowed). This allows callers to release without tracking whether
    /// they acquired the borrow.
    pub fn release_read(&self, type_id: TypeId) -> bool {
        let borrows = self.borrows.read().unwrap();
        if let Some(atomic) = borrows.get(&type_id) {
            let current = atomic.load(Ordering::Acquire);
            if current > 0 {
                atomic.store(current - 1, Ordering::Release);
                return true;
            }
        }
        false
    }

    /// Acquires an exclusive (write) borrow on a component type.
    ///
    /// # Panics
    ///
    /// Panics if there is any active borrow (read or write) on the same type.
    pub fn borrow_write(&self, type_id: TypeId) {
        self.ensure_registered(type_id);
        let borrows = self.borrows.read().unwrap();
        let atomic = borrows.get(&type_id).unwrap();
        let current = atomic.load(Ordering::Acquire);
        assert!(
            current == 0,
            "cannot borrow component as mutable: already borrowed"
        );
        atomic.store(-1, Ordering::Release);
    }

    /// Releases an exclusive (write) borrow on a component type.
    ///
    /// Returns `false` if there was no active write borrow.
    pub fn release_write(&self, type_id: TypeId) -> bool {
        let borrows = self.borrows.read().unwrap();
        if let Some(atomic) = borrows.get(&type_id) {
            if atomic.load(Ordering::Acquire) == -1 {
                atomic.store(0, Ordering::Release);
                return true;
            }
        }
        false
    }
}

impl Default for AccessTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
