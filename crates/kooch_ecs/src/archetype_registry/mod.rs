//! Archetype registry — tracks entity-to-archetype mappings and caches
//! archetype transitions.
//!
//! [`ArchetypeRegistry`] is the central index that knows which archetype
//! every entity belongs to and provides efficient archetype-based iteration.

mod registry;

#[cfg(test)]
mod tests;

pub use registry::ArchetypeRegistry;
