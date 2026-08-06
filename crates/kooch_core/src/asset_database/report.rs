/// Summary returned by [`super::AssetDatabase::scan_directory`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ScanReport {
    /// Files that had no `.meta` and got one, because a registered
    /// loader claims their extension. Zero on a project whose assets all
    /// came through the editor; non-zero the first time a scan meets a
    /// file someone wrote by hand.
    pub adopted: usize,
    /// Number of `(guid, path)` pairs successfully registered.
    pub registered: usize,
    /// Sidecars whose source file was missing (orphans). Logged but
    /// not registered; the database refuses to point at nothing.
    pub orphans: usize,
    /// Sidecars that already existed in the database (same path, same
    /// GUID). Idempotent re-scan path.
    pub duplicates: usize,
}
