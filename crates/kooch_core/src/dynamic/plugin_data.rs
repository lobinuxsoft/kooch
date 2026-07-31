//! Key-value byte storage for inter-plugin communication.
//!
//! Plugins can't share Rust types across the FFI boundary (different
//! allocators, no shared `TypeId`). Instead, they exchange raw bytes
//! keyed by string names through [`PluginData`].

use std::collections::HashMap;

/// Key-value store of raw bytes for plugin communication.
///
/// Stored as a resource in [`Resources`](crate::resource::Resources).
/// Accessed through [`EngineApi::set_data`](kooch_plugin_api::EngineApi::set_data)
/// and [`EngineApi::get_data`](kooch_plugin_api::EngineApi::get_data).
pub struct PluginData {
    store: HashMap<String, Vec<u8>>,
}

impl PluginData {
    /// Creates an empty store.
    pub fn new() -> Self {
        Self {
            store: HashMap::new(),
        }
    }

    /// Stores data under a key, replacing any existing value.
    pub fn set(&mut self, key: &str, data: &[u8]) {
        self.store.insert(key.to_owned(), data.to_vec());
    }

    /// Returns the data stored under a key, if any.
    pub fn get(&self, key: &str) -> Option<&[u8]> {
        self.store.get(key).map(|v| v.as_slice())
    }

    /// Removes the data stored under a key.
    pub fn remove(&mut self, key: &str) -> Option<Vec<u8>> {
        self.store.remove(key)
    }

    /// Returns the number of stored entries.
    #[inline]
    pub fn len(&self) -> usize {
        self.store.len()
    }

    /// Returns `true` if no entries are stored.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.store.is_empty()
    }
}

impl Default for PluginData {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_and_get() {
        let mut pd = PluginData::new();
        pd.set("key", b"value");

        assert_eq!(pd.get("key"), Some(b"value".as_slice()));
        assert_eq!(pd.get("missing"), None);
    }

    #[test]
    fn overwrite() {
        let mut pd = PluginData::new();
        pd.set("k", b"first");
        pd.set("k", b"second");

        assert_eq!(pd.get("k"), Some(b"second".as_slice()));
        assert_eq!(pd.len(), 1);
    }

    #[test]
    fn remove() {
        let mut pd = PluginData::new();
        pd.set("k", b"data");

        let removed = pd.remove("k");
        assert_eq!(removed, Some(b"data".to_vec()));
        assert!(pd.is_empty());
    }

    #[test]
    fn empty_data() {
        let mut pd = PluginData::new();
        pd.set("empty", b"");

        assert_eq!(pd.get("empty"), Some(b"".as_slice()));
    }
}
