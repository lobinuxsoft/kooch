//! Main `Query` type and iterator.

mod core;
mod iter;

#[cfg(test)]
mod tests;

pub use core::Query;
pub use iter::QueryIter;

#[cfg(test)]
mod bench;
