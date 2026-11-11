//! Project-wide asset registry — `Guid ↔ path` bidirectional map.

mod database;
mod entry;
mod error;
mod report;
mod scan;

#[cfg(test)]
mod tests;

pub use database::AssetDatabase;

/// A number that moves whenever an asset is registered, removed or rewritten: a system that reads
/// assets on load does nothing while it holds still (#1882).
pub fn changes(resources: &crate::resource::Resources) -> u64 {
    let database = resources
        .get::<AssetDatabase>()
        .map_or(0, AssetDatabase::revision);
    let rewritten = resources
        .get::<crate::asset_loader::ReloadedAssets>()
        .map_or(0, crate::asset_loader::ReloadedAssets::last_write);
    database.max(rewritten)
}
pub use entry::AssetEntry;
pub use error::AssetDatabaseError;
pub use report::ScanReport;
