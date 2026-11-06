use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{flatten, used_files};

fn reader(files: &[(&str, &str)]) -> impl FnMut(&Path) -> Result<Vec<u8>, String> + use<> {
    let files: HashMap<PathBuf, Vec<u8>> = files
        .iter()
        .map(|(name, text)| (PathBuf::from(name), text.as_bytes().to_vec()))
        .collect();
    move |path| files.get(path).cloned().ok_or_else(|| "missing".to_owned())
}

#[test]
fn a_use_cycle_ends() {
    let mut read = reader(&[
        ("/a/b.wgsl", "// use: c.wgsl\nfn b() {}"),
        (
            "/a/c.wgsl",
            "// use: b.wgsl\n// use: ../a/main.wgsl\nfn c() {}",
        ),
    ]);
    let source = "// use: b.wgsl\nfn main() {}";
    let used = used_files(Path::new("/a/main.wgsl"), source, &mut read).unwrap();
    assert_eq!(used.len(), 2);
}

#[test]
fn a_shared_file_appends_once() {
    let mut read = reader(&[
        ("/a/b.wgsl", "// use: lib/d.wgsl\nfn b() {}"),
        ("/a/c.wgsl", "// use: lib/../lib/d.wgsl\nfn c() {}"),
        ("/a/lib/d.wgsl", "fn d() {}"),
    ]);
    let source = "// use: b.wgsl\n// use: c.wgsl\nfn main() {}";
    let used = used_files(Path::new("/a/main.wgsl"), source, &mut read).unwrap();
    assert_eq!(flatten(source, &used).matches("fn d()").count(), 1);
}

#[test]
fn a_graph_stays_behind() {
    let used = [(
        PathBuf::from("/a/b.subgraph"),
        "// kind: ui\n/*KOOCH_GRAPH;(nodes)*/\nfn b() {}\n/*KOOCH_NOTES;()*/\n".to_owned(),
    )];
    let flat = flatten("fn main() {}", &used);
    assert!(flat.contains("fn b()") && !flat.contains("KOOCH"));
}

#[test]
fn a_missing_file_is_refused() {
    let mut read = reader(&[]);
    let result = used_files(Path::new("/a/main.wgsl"), "// use: b.wgsl\n", &mut read);
    assert!(result.is_err());
}

#[test]
fn the_library_needs_no_file() {
    let mut read = reader(&[]);
    let source = "// use: engine:shapes\n";
    let used = used_files(Path::new("/a/main.wgsl"), source, &mut read).unwrap();
    assert!(flatten(source, &used).contains("fn lib_box"));
}
