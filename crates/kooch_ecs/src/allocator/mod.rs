//! Entity allocator with generational indices and GPU sync tracking.
//!
//! Uses a FIFO free-list (`VecDeque`) so recycled slots get maximum
//! temporal separation before reuse, reducing the chance of stale
//! references going undetected.

#[allow(clippy::module_inception)]
mod allocator;

#[cfg(test)]
mod tests;

pub use allocator::EntityAllocator;
