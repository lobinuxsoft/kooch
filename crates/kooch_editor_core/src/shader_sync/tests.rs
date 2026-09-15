use std::time::{Duration, SystemTime};

use std::path::PathBuf;

use super::ShaderSync;

#[test]
fn a_first_sighting_is_not_a_change() {
    let mut sync = ShaderSync::default();
    assert!(!sync.moved("a.shader".into(), SystemTime::UNIX_EPOCH));
}

#[test]
fn a_new_mtime_is_a_change() {
    let mut sync = ShaderSync::default();
    sync.moved("a.shader".into(), SystemTime::UNIX_EPOCH);
    let later = SystemTime::UNIX_EPOCH + Duration::from_secs(1);
    assert!(sync.moved("a.shader".into(), later));
    assert!(!sync.moved("a.shader".into(), later));
}

fn project() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "kooch-vscode-settings-{}",
        kooch_core::Guid::new_v4()
    ));
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn parsed(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap()
}

#[test]
fn a_project_without_settings_gets_them() {
    let root = project();
    super::write_vscode_settings(&root);
    let settings = parsed(&std::fs::read_to_string(root.join(".vscode/settings.json")).unwrap());
    assert_eq!(settings["files.associations"]["*.shader"], "wgsl");
    for key in super::ANALYZER_SETTINGS {
        assert_eq!(settings[key], false, "{key}");
    }
    std::fs::remove_dir_all(root).unwrap();
}

/// The author's keys stay, in their order and formatting; only the missing ones are added.
#[test]
fn existing_settings_are_extended() {
    let text =
        "{\n    \"editor.fontSize\": 14,\n    \"files.associations\": { \"*.foo\": \"toml\" }\n}\n";
    let merged = super::merged_settings(text)
        .unwrap()
        .expect("something was missing");
    assert!(
        merged.starts_with("{\n    \"editor.fontSize\": 14,"),
        "{merged}"
    );
    let settings = parsed(&merged);
    assert_eq!(settings["editor.fontSize"], 14);
    assert_eq!(settings["files.associations"]["*.foo"], "toml");
    assert_eq!(settings["files.associations"]["*.shader"], "wgsl");
    assert_eq!(settings["wgsl-analyzer.inlayHints.typeHints"], false);
}

/// A value the author chose is theirs, even the opposite of ours.
#[test]
fn an_explicit_choice_is_kept() {
    let text = r#"{ "wgsl-analyzer.diagnostics.typeErrors": true }"#;
    let merged = super::merged_settings(text).unwrap().unwrap();
    assert_eq!(
        parsed(&merged)["wgsl-analyzer.diagnostics.typeErrors"],
        true
    );
}

#[test]
fn complete_settings_are_not_rewritten() {
    let complete = super::merged_settings("{}").unwrap().unwrap();
    assert_eq!(super::merged_settings(&complete).unwrap(), None);
}

/// Comments are legal in VS Code and not in JSON: refused, never mangled.
#[test]
fn commented_settings_are_refused() {
    let root = project();
    let text = "{\n  // mine\n  \"editor.fontSize\": 14\n}\n";
    std::fs::create_dir_all(root.join(".vscode")).unwrap();
    std::fs::write(root.join(".vscode/settings.json"), text).unwrap();
    super::write_vscode_settings(&root);
    assert_eq!(
        std::fs::read_to_string(root.join(".vscode/settings.json")).unwrap(),
        text
    );
    std::fs::remove_dir_all(root).unwrap();
}
