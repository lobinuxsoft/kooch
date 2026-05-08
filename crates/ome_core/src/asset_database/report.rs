/// Summary returned by [`super::AssetDatabase::scan_directory`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ScanReport {
    /// Number of `(guid, path)` pairs successfully registered.
    pub registered: usize,
    /// Sidecars whose source file was missing (orphans). Logged but
    /// not registered; the database refuses to point at nothing.
    pub orphans: usize,
    /// Sidecars that already existed in the database (same path, same
    /// GUID). Idempotent re-scan path.
    pub duplicates: usize,
}
