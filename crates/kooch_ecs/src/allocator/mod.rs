//! Entity allocator with generational indices and GPU sync tracking.

#[allow(clippy::module_inception)]
mod allocator;

#[cfg(test)]
mod tests;

pub use allocator::EntityAllocator;
