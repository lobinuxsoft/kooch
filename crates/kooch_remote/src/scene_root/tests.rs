use super::*;

/// A fresh project folder under the temp dir, with one scene in it.
fn project(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("kooch_scene_root_{name}"));
    std::fs::create_dir_all(root.join("scenes")).unwrap();
    std::fs::write(root.join("scenes/main.scene"), "").unwrap();
    root
}

#[test]
fn a_project_scene_is_allowed() {
    let root = project("inside");
    assert!(confine(&root, Path::new("scenes/main.scene")).is_ok());
    assert!(confine(&root, Path::new("scenes/new/unsaved.scene")).is_ok());
    assert!(confine(&root, &root.join("scenes/main.scene")).is_ok());
}

/// Anything on the socket must not get a write outside the project (#647).
#[test]
fn an_escaping_path_is_refused() {
    let root = project("outside");
    assert!(confine(&root, Path::new("../elsewhere.scene")).is_err());
    assert!(confine(&root, Path::new("scenes/new/../../../x.scene")).is_err());
    assert!(confine(&root, Path::new("/etc/passwd")).is_err());
}
