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

#[test]
fn settings_are_written_once() {
    let root = project();
    super::write_vscode_settings(&root);
    let settings = std::fs::read_to_string(root.join(".vscode/settings.json")).unwrap();
    assert!(settings.contains("\"*.shader\": \"wgsl\""), "{settings}");
    assert!(
        settings.contains("\"wgsl-analyzer.diagnostics.typeErrors\": false"),
        "{settings}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

/// Settings the author already has are never touched.
#[test]
fn existing_settings_are_kept() {
    let root = project();
    std::fs::create_dir_all(root.join(".vscode")).unwrap();
    std::fs::write(root.join(".vscode/settings.json"), "{}").unwrap();
    super::write_vscode_settings(&root);
    assert_eq!(
        std::fs::read_to_string(root.join(".vscode/settings.json")).unwrap(),
        "{}"
    );
    std::fs::remove_dir_all(root).unwrap();
}
