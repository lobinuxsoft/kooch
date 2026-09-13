//! Dense component storage — the columns a table is made of.

pub mod column;
pub mod table;
pub mod tables;

pub use column::Column;
pub use table::{Table, TableRow};
pub use tables::{TableId, Tables};
