//! Type-safe query system for ECS components.
//!
//! Provides [`Query`] for iterating over entities with specific component
//! patterns, with runtime borrow checking to prevent data races.
//!
//! # Architecture
//!
//! - **CPU Queries** — Immediate access to both CPU and GPU components
//!   (GPU components via last sync'd CPU shadow copy).
//! - **Filters** — [`With<T>`], [`Without<T>`] for archetype-level filtering.
//! - **Runtime safety** — [`AccessTracker`] prevents conflicting borrows
//!   (multiple `&mut` on the same component type).
//!
//! # Example
//!
//! ```ignore
//! use ome_ecs::query::{Query, With, Without};
//!
//! fn gameplay_system(resources: &mut Resources) {
//!     // Iterate entities with Position and Velocity, excluding Dead
//!     let query = Query::<(&Position, &mut Velocity), Without<Dead>>::new(resources);
//!     for (pos, vel) in query.iter() {
//!         // pos: &Position, vel: &mut Velocity
//!     }
//! }
//! ```

mod access;
pub mod fetch;
pub mod filter;
pub mod query;

pub use access::AccessTracker;
pub use fetch::WorldQuery;
pub use filter::{QueryFilter, With, Without};
pub use query::{Query, QueryIter};
