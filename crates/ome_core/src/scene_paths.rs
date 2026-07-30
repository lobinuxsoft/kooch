//! What scene and prefab files are called, in one place.
//!
//! # Why these live here rather than beside the code that uses them
//!
//! Three crates need to agree about these names: the editor writes them,
//! the runtime's scene bootstrap looks for them, and a project's manifest
//! records one. The editor cannot own them — the runtime would have to
//! depend on the editor to read them, which is backwards — so they sat
//! duplicated in both, and a rename changed one copy. The runtime then
//! went looking for a file the editor no longer wrote, and the
//! self-healing default-scene path wrote a fresh empty one over the gap.
//!
//! `ome_core` is the one crate both already depend on.

/// Extension of a scene file.
pub const SCENE_EXTENSION: &str = "scene";

/// Extension of a prefab file.
///
/// # Why a second extension for one format
///
/// A prefab and a scene are the same document written by the same
/// serialiser — see
/// [`SceneDocument::from_ecs_subtree`](../../ome_ecs/scene/document/struct.SceneDocument.html)
/// for why a second format would be a mistake. What differs is an
/// **invariant**: a prefab has exactly one root entity, a scene may have
/// any number.
///
/// With one extension there was nothing to tell them apart, so the editor
/// offered "instantiate" on every scene file and a four-root scene failed
/// at the point of instancing — a check the user had no way to anticipate.
/// The extension is what makes the invariant visible before the click, and
/// lets it be enforced when the file is *written* instead.
///
/// Unity draws the same line the same way: `.unity` and `.prefab` hold the
/// same YAML.
///
/// # Why not `ome_`-prefixed
///
/// The prefix was namespacing against a collision that does not happen —
/// these files sit in a project's own `scenes/` and `assets/`, not in a
/// shared folder — and it made the *engine* the visible thing about a file
/// whose interesting property is its format.
pub const PREFAB_EXTENSION: &str = "prefab";

/// Convention path of a project's default scene, relative to its root.
///
/// Also the path the runtime falls back to relative to the working
/// directory when no `--scene` was passed; see the cwd caveat on
/// `SceneBootstrapPlugin`.
pub const DEFAULT_SCENE_REL_PATH: &str = "scenes/default.scene";

#[cfg(test)]
mod tests {
    use super::*;

    /// The default path and the extension were two literals in two crates,
    /// and a rename moved one. They are one fact now, and this fails if
    /// they ever drift apart again.
    #[test]
    fn the_default_scene_path_carries_the_scene_extension() {
        assert!(
            DEFAULT_SCENE_REL_PATH.ends_with(&format!(".{SCENE_EXTENSION}")),
            "{DEFAULT_SCENE_REL_PATH} does not end in .{SCENE_EXTENSION}",
        );
    }

    /// The whole point of two extensions is that they differ.
    #[test]
    fn a_scene_and_a_prefab_are_told_apart() {
        assert_ne!(SCENE_EXTENSION, PREFAB_EXTENSION);
    }

    /// A leading dot would make every `format!(".{ext}")` produce `..scene`.
    #[test]
    fn an_extension_is_bare() {
        for ext in [SCENE_EXTENSION, PREFAB_EXTENSION] {
            assert!(!ext.starts_with('.'), "{ext} should not carry its dot");
            assert!(!ext.is_empty());
        }
    }
}
