//! Type-erased component registry.
//!
//! [`ComponentRegistry`] maps `TypeId` → `Box<dyn AnyStorage>`, providing
//! typed access via downcasting and batch operations (remove entity from
//! all storages, sync all GPU storages).

mod core;

#[cfg(test)]
mod tests;

pub use core::ComponentRegistry;
