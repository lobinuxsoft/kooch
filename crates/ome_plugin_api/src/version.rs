//! Plugin API versioning.
//!
//! Plugins report their API version via [`OmePlugin::api_version`](crate::OmePlugin::api_version).
//! The engine rejects plugins built against incompatible versions.

/// Current plugin API version.
///
/// Increment this on any breaking change to `EngineApi` or `OmePlugin`.
pub const API_VERSION: u32 = 1;

/// Returns `true` if `plugin_version` is compatible with the engine.
///
/// Currently requires an exact match. Future versions may support
/// backwards-compatible ranges.
#[inline]
pub const fn is_compatible(plugin_version: u32) -> bool {
    plugin_version == API_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_version_compatible() {
        assert!(is_compatible(API_VERSION));
    }

    #[test]
    fn wrong_version_rejected() {
        assert!(!is_compatible(0));
        assert!(!is_compatible(API_VERSION + 1));
    }
}
