//! Project-wide asset registry — `Guid ↔ path` bidirectional map.

mod database;
mod entry;
mod error;
mod report;
mod scan;

#[cfg(test)]
mod tests;

pub use database::AssetDatabase;
pub use entry::AssetEntry;
pub use error::AssetDatabaseError;
pub use report::ScanReport;
