/// 🔴 The case this variant exists for, and the one that used to be
/// silent: same API, same compiler, different engine. The plugin
/// links and reads every shared structure at whatever layout it was
/// compiled with.
///
/// It stopped being hypothetical when the engine started being
/// vendored into projects (#754) — the editor's copy and a project's
/// copy are two directories that can drift.
#[test]
fn a_plugin_from_another_engine_version_is_refused() {
    let stale = super::BuildStamp {
        engine_hash: super::BuildStamp::current().engine_hash ^ 1,
        ..super::BuildStamp::current()
    };

    assert!(!stale.is_compatible_with_current());
    assert!(matches!(
        stale.incompatibility(),
        Some(super::Incompatibility::EngineVersion { .. }),
    ));
}

/// The three checks answer three different questions, and the
/// loader reports which one failed because they have different
/// fixes: bump the API, change toolchain, rebuild the project.
#[test]
fn each_half_of_the_stamp_is_reported_separately() {
    let current = super::BuildStamp::current();
    let with = |f: fn(&mut super::BuildStamp)| {
        let mut s = current;
        f(&mut s);
        s.incompatibility()
    };

    assert!(matches!(
        with(|s| s.api_version += 1),
        Some(super::Incompatibility::ApiVersion { .. }),
    ));
    assert!(matches!(
        with(|s| s.rustc_hash ^= 1),
        Some(super::Incompatibility::Compiler),
    ));
    assert!(matches!(
        with(|s| s.engine_hash ^= 1),
        Some(super::Incompatibility::EngineVersion { .. }),
    ));
    assert!(current.incompatibility().is_none());
}

use super::*;

#[test]
fn a_plugin_built_here_is_compatible() {
    assert!(BuildStamp::current().is_compatible_with_current());
    assert_eq!(BuildStamp::current().incompatibility(), None);
}

#[test]
fn a_different_api_version_names_itself() {
    let stamp = BuildStamp {
        api_version: API_VERSION + 1,
        ..BuildStamp::current()
    };
    assert!(!stamp.is_compatible_with_current());
    assert_eq!(
        stamp.incompatibility(),
        Some(Incompatibility::ApiVersion {
            plugin: API_VERSION + 1,
            engine: API_VERSION,
        })
    );
}

/// The two failures need different fixes, so they must not collapse
/// into one message.
#[test]
fn a_different_compiler_is_reported_separately() {
    let stamp = BuildStamp {
        rustc_hash: BuildStamp::current().rustc_hash ^ 0xFFFF,
        ..BuildStamp::current()
    };
    assert_eq!(stamp.incompatibility(), Some(Incompatibility::Compiler));
}

#[test]
fn the_compiler_identity_was_captured() {
    assert!(
        !RUSTC_IDENT.is_empty(),
        "build script must record a compiler identity"
    );
    assert_ne!(BuildStamp::current().rustc_hash, 0);
}

#[test]
fn fnv1a_separates_similar_inputs() {
    assert_ne!(fnv1a(b"rustc 1.93.0"), fnv1a(b"rustc 1.94.0"));
    assert_eq!(fnv1a(b"same"), fnv1a(b"same"));
}
