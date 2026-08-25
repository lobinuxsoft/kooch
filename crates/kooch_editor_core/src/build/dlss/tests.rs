use super::*;

fn preset_with(features: &str) -> BuildPreset {
    BuildPreset {
        features: features.to_owned(),
        ..Default::default()
    }
}

#[test]
fn only_the_exact_feature_counts() {
    assert!(wanted(&preset_with("kooch/dlss")));
    assert!(wanted(&preset_with("audio, kooch/dlss")));
    assert!(!wanted(&preset_with("")));
    assert!(!wanted(&preset_with("audio")));
}

/// 🔴 `dlss` on its own is a feature of the GAME's crate, and cargo
/// refuses a build that names one the project never declared. The
/// namespaced spelling is what a preset should carry — and the bare one
/// is what everybody types, so both are honoured here.
#[test]
fn the_bare_spelling_is_honoured_too() {
    assert!(wanted(&preset_with("dlss")));
    assert!(wanted(&preset_with("audio, dlss")));
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
    assert_eq!(
        ship(&preset_with(""), super::super::Platform::Linux, &dir).unwrap(),
        Vec::<PathBuf>::new()
    );
}

const MANIFEST: &str = "[package]
name = \"a_game\"

[features]
default = [\"game\"]
game = [\"kooch/physics\"]

[dependencies]
kooch = { path = \"../kooch\" }
";

#[test]
fn a_project_without_its_own_gets_the_prefix() {
    assert!(!declares_feature(MANIFEST, BARE_FEATURE));
}

/// 🔴 The guard. A project that declares `dlss` means its own, and
/// rewriting it would build something the author did not ask for.
#[test]
fn a_project_with_its_own_keeps_it() {
    let manifest = MANIFEST.replace("game = ", "dlss = [\"kooch/dlss\"]\ngame = ");
    assert!(declares_feature(&manifest, BARE_FEATURE));
}

#[test]
fn a_feature_outside_the_table_does_not_count() {
    assert!(!declares_feature(
        "[dependencies]\ndlss = \"1\"\n",
        BARE_FEATURE
    ));
}

#[test]
fn a_commented_out_feature_does_not_count() {
    assert!(!declares_feature("[features]\n# dlss = []\n", BARE_FEATURE));
}

#[test]
fn the_untouched_features_travel_unchanged() {
    let dir = std::env::temp_dir();
    let features = vec!["audio".to_owned(), "kooch/dlss".to_owned()];
    assert_eq!(normalise(features.clone(), &dir), features);
}

/// 🔴 The header lives under `include/vulkan/`, not at the root of the
/// SDK. Getting that wrong is how the check passes and the build still
/// dies in bindgen.
#[test]
fn the_header_sits_under_include() {
    let path = header_in(Path::new("/usr"));
    assert!(path.ends_with("vulkan/vulkan.h"), "{}", path.display());
    assert!(path.starts_with("/usr"));
}

/// Whatever this machine has, the answer must be a directory that
/// actually holds the header bindgen could not find — never a guess.
#[test]
fn the_clang_include_holds_what_it_promises() {
    if let Some(include) = clang_include() {
        assert!(include.join("stdbool.h").is_file(), "{}", include.display());
    }
}
