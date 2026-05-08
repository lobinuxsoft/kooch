//! Deferred command buffer for entity spawn/despawn and component operations.
//!
//! [`Commands`] collects mutation requests during system execution and applies
//! them in a single batch, preventing iterator invalidation and race conditions.
//!
//! Entity allocation is **immediate** (via [`EntityAllocator`]) so callers get
//! a valid [`Entity`] ID right away. Component insertion, removal, and despawn
//! are **deferred** until [`Commands::apply`] (or the built-in apply system).
//!
//! # Example
//!
//! ```ignore
//! fn setup_system(resources: &mut Resources) {
//!     let mut commands = Commands::new();
//!     let player = commands.spawn(resources)
//!         .insert(Health(100))
//!         .insert(Name("Player".into()))
//!         .id();
//!     commands.apply(resources);
//! }
//! ```
//!
//! [`EntityAllocator`]: crate::allocator::EntityAllocator
//! [`Entity`]: crate::entity::Entity

mod buffer;
mod command;
mod entity_builder;
mod entity_commands;

#[cfg(test)]
mod tests;

use ome_core::resource::Resources;

pub use buffer::Commands;
pub use entity_builder::EntityBuilder;
pub use entity_commands::EntityCommands;

/// System that applies all pending [`Commands`].
///
/// Should run in [`Stage::GpuSync`](ome_core::stage::Stage::GpuSync) **before**
/// the despawn cleanup system so that newly spawned entities get their
/// components before GPU sync.
pub fn commands_apply_system(resources: &mut Resources) {
    let mut commands = match resources.remove::<Commands>() {
        Some(c) => c,
        None => return,
    };
    commands.apply(resources);
    resources.insert(commands);
}
