//! A dense handle for a registered component type.

/// Which slot of the [`ComponentRegistry`](crate::component::ComponentRegistry) a component type
/// occupies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StorageId(pub u32);

impl StorageId {
    /// The slot this id indexes.
    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}
