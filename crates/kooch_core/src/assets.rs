//! Typed asset handle system.
//!
//! Provides type-safe references to loaded assets (meshes, textures,
//! materials, sounds). Game code stores [`Handle<T>`] values; the actual
//! data lives in [`Assets<T>`] resources keyed by asset type.
//!
//! # DOD layout
//!
//! Storage is a `slotmap::SlotMap<DefaultKey, T>` — a generational arena
//! with `Vec<T>` dense storage and `u32` indices. O(1) insert / get /
//! remove. Stale handles (where `remove + insert` reused a slot) are
//! detected via the embedded generation counter and return `None`.
//!
//! # Usage
//!
//! ```ignore
//! use kooch_core::assets::{Asset, Assets, Handle};
//!
//! struct Mesh { /* ... */ }
//! impl Asset for Mesh {}
//!
//! let mut meshes = Assets::<Mesh>::new();
//! let handle: Handle<Mesh> = meshes.insert(Mesh { /* ... */ });
//! let mesh_ref: &Mesh = meshes.get(handle).unwrap();
//! ```
//!
//! Each asset type lives in its own `Assets<T>` instance, typically inserted
//! as a [`Resource`](crate::resource::Resources) at engine startup.
//!
//! # Future work (out of scope for #184)
//!
//! - `AssetServer` (path → handle resolver) lives with the loader trait
//!   in #391.
//! - Scene serialization of handles as paths lands when loaders exist.
//! - Reference counting / auto-cleanup is intentionally absent — assets
//!   are explicitly removed; no automatic GC.

use slotmap::{DefaultKey, SlotMap};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

/// Marker trait for types that can be stored as assets.
///
/// Implementing types must be `Send + Sync + 'static`. The blanket impl
/// makes any qualifying type usable without manual opt-in.
pub trait Asset: Send + Sync + 'static {}

impl<T: Send + Sync + 'static> Asset for T {}

/// Typed handle to an asset stored in [`Assets<T>`].
///
/// Cheap to copy: a `slotmap::DefaultKey` (16 bytes — index + generation)
/// plus a zero-sized marker. Handles are *not* reference-counted; removing
/// the asset from `Assets<T>` invalidates outstanding handles, which then
/// return `None` on `get`.
///
/// `Handle<Mesh>` and `Handle<Texture>` are distinct types — passing one
/// where the other is expected fails at compile time.
pub struct Handle<T: Asset> {
    key: DefaultKey,
    _marker: PhantomData<fn() -> T>,
}

impl<T: Asset> Handle<T> {
    /// Returns the underlying slotmap key. Mostly for debugging / logging.
    #[inline]
    pub fn key(&self) -> DefaultKey {
        self.key
    }

    /// Constructs a handle from a raw key. Caller must ensure the key
    /// originated from an `Assets<T>` of the same type — there is no
    /// runtime check.
    #[inline]
    pub fn from_key(key: DefaultKey) -> Self {
        Self {
            key,
            _marker: PhantomData,
        }
    }
}

// PhantomData<fn() -> T> makes Handle covariant + free of T-trait bounds
// for Clone/Copy/Eq/Hash/Debug derivations — manual impls follow.

impl<T: Asset> Clone for Handle<T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Asset> Copy for Handle<T> {}

impl<T: Asset> PartialEq for Handle<T> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl<T: Asset> Eq for Handle<T> {}

impl<T: Asset> Hash for Handle<T> {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key.hash(state);
    }
}

impl<T: Asset> fmt::Debug for Handle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Handle")
            .field(&std::any::type_name::<T>())
            .field(&self.key)
            .finish()
    }
}

/// Storage for assets of type `T`.
///
/// Inserted as a [`Resource`](crate::resource::Resources) — typically one
/// instance per asset type. Loaders write here; render systems read.
pub struct Assets<T: Asset> {
    storage: SlotMap<DefaultKey, T>,
}

impl<T: Asset> Assets<T> {
    /// Creates an empty store.
    #[inline]
    pub fn new() -> Self {
        Self {
            storage: SlotMap::new(),
        }
    }

    /// Inserts an asset, returning a handle to retrieve it later.
    pub fn insert(&mut self, asset: T) -> Handle<T> {
        Handle {
            key: self.storage.insert(asset),
            _marker: PhantomData,
        }
    }

    /// Returns a shared reference to the asset, or `None` if the handle
    /// no longer points to a live slot (asset was removed, or the slot
    /// was reused with a different generation).
    #[inline]
    pub fn get(&self, handle: Handle<T>) -> Option<&T> {
        self.storage.get(handle.key)
    }

    /// Returns a mutable reference to the asset, or `None` if invalid.
    #[inline]
    pub fn get_mut(&mut self, handle: Handle<T>) -> Option<&mut T> {
        self.storage.get_mut(handle.key)
    }

    /// Removes the asset and returns it. Returns `None` if the handle
    /// is invalid. The freed slot is reused on a future `insert` with a
    /// new generation; pre-existing copies of the old handle become
    /// stale and return `None` from `get`.
    pub fn remove(&mut self, handle: Handle<T>) -> Option<T> {
        self.storage.remove(handle.key)
    }

    /// `true` when the handle points to a live asset.
    #[inline]
    pub fn contains(&self, handle: Handle<T>) -> bool {
        self.storage.contains_key(handle.key)
    }

    /// Number of live assets.
    #[inline]
    pub fn len(&self) -> usize {
        self.storage.len()
    }

    /// `true` when no assets are stored.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.storage.is_empty()
    }

    /// Iterates over all live `(handle, asset)` pairs in arbitrary order.
    pub fn iter(&self) -> impl Iterator<Item = (Handle<T>, &T)> + '_ {
        self.storage
            .iter()
            .map(|(key, asset)| (Handle::from_key(key), asset))
    }

    /// Mutable variant of [`iter`](Self::iter).
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Handle<T>, &mut T)> + '_ {
        self.storage
            .iter_mut()
            .map(|(key, asset)| (Handle::from_key(key), asset))
    }

    /// Removes every asset, leaving the store empty.
    pub fn clear(&mut self) {
        self.storage.clear();
    }
}

impl<T: Asset> Default for Assets<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
