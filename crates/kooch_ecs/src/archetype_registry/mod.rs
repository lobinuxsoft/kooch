//! Archetype registry — tracks entity-to-archetype mappings and caches archetype transitions.

mod registry;

#[cfg(test)]
mod tests;

pub use registry::ArchetypeRegistry;
