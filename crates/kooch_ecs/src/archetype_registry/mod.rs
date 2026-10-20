//! Archetype registry — tracks entity-to-archetype mappings and caches archetype transitions.

mod filed;
mod registry;

#[cfg(test)]
mod tests;

pub use filed::Filed;
pub use registry::{ArchetypeRegistry, Where};
