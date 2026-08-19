//! A dense handle for a registered component type.

/// Which slot of the [`ComponentRegistry`] a component type occupies.
///
/// Dense, `Copy`, and minted on first registration — so it can index a
/// `Vec` directly. That is the whole point: a `TypeId` is a 128-bit hash
/// and a column cannot be indexed by one.
///
/// # Not to be confused with [`ComponentId`]
///
/// [`ComponentId`] interns a component's **name** for the editor and the
/// remote protocol, and it mints ids for names that were never registered
/// as storages here. This one is the registry's own index and its identity
/// is the Rust type. They are unrelated numbers and neither converts to
/// the other.
///
/// # Never serialize it
///
/// It depends on registration order, which depends on which plugins ran.
/// Two processes will disagree. The wire format carries the name.
///
/// [`ComponentRegistry`]: crate::component::ComponentRegistry
/// [`ComponentId`]: crate::component::ComponentId
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StorageId(pub u32);

impl StorageId {
    /// The slot this id indexes.
    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}
