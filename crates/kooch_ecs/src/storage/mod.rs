//! Dense component storage — the columns a table is made of.
//!
//! See #891: an archetype says *who* has the components, and this layer
//! says *where the values are*, by row rather than by key.

pub mod column;

pub use column::Column;
