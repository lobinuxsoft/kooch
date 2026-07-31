//! Main `Query` type and iterator.
//!
//! [`Query`] is constructed from [`Resources`] and provides ergonomic,
//! type-safe iteration over entities matching a component + filter pattern.
//!
//! [`Resources`]: kooch_core::resource::Resources

mod core;
mod iter;

#[cfg(test)]
mod tests;

pub use core::Query;
pub use iter::QueryIter;
