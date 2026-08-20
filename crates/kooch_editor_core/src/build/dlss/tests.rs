use super::*;

fn preset_with(features: &str) -> BuildPreset {
    BuildPreset {
        features: features.to_owned(),
        ..Default::default()
    }
}

#[test]
fn only_the_exact_feature_counts() {
    assert!(wanted(&preset_with("dlss")));
    assert!(wanted(&preset_with("audio, dlss")));
    assert!(!wanted(&preset_with("")));
    assert!(!wanted(&preset_with("audio")));
}

/// 🔴 The failure this guards against is a preset whose features happen
/// to mention DLSS in passing pulling in an SDK requirement nobody
/// asked for.
#[test]
fn a_similar_name_is_not_the_feature() {
    assert!(!wanted(&preset_with("dlss-off")));
    assert!(!wanted(&preset_with("my_dlss")));
}

#[test]
fn a_preset_without_the_feature_needs_no_sdk() {
    assert_eq!(missing_sdk(&preset_with("audio")), None);
}

/// Nothing is copied for a build that never asked, whether or not this
/// machine has an SDK.
#[test]
fn a_preset_without_the_feature_ships_nothing() {
    let dir = std::env::temp_dir();
    assert_eq!(ship(&preset_with(""), &dir).unwrap(), Vec::<PathBuf>::new());
}
