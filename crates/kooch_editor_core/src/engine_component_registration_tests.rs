/// 🔴 Every `*ComponentsPlugin` in the workspace is added by the
/// editor.
///
/// The editor keeps its **own** `ComponentRegistry`. A component the
/// project registers is invisible here, so authoring one requires the
/// matching components-plugin in `EditorPlugin::build` — and the list
/// is written by hand, which is exactly as reliable as it sounds:
/// this has now been the fifth omission (#722 was the third,
/// `InputMapSource` the fifth), and each one surfaces as a component
/// the menu offers and then refuses with "no default value".
///
/// Scanning the source rather than a registry because the failure is
/// a plugin that was never *added* — a runtime check could only see
/// what was added, which is the set that is already correct.
#[test]
fn every_components_plugin_is_added_by_the_editor() {
    let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/ dir");
    let lib = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"),
    )
    .expect("read the editor's own lib.rs");

    let mut found = Vec::new();
    collect_components_plugins(crates, &mut found);
    assert!(
        found.len() >= 4,
        "the scan found {} plugins, so it is not scanning: {found:?}",
        found.len()
    );

    for name in &found {
        assert!(
            lib.contains(name.as_str()),
            "{name} exists but `EditorPlugin::build` never adds it, so its \
                 components cannot be authored — the add-component menu will \
                 offer them and fail with \"no default value\"",
        );
    }
}

/// Every `pub struct <X>ComponentsPlugin` under `dir`.
fn collect_components_plugins(dir: &std::path::Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            collect_components_plugins(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            for line in text.lines() {
                let line = line.trim();
                let Some(rest) = line.strip_prefix("pub struct ") else {
                    continue;
                };
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if name.ends_with("ComponentsPlugin") && !out.contains(&name) {
                    out.push(name);
                }
            }
        }
    }
}
