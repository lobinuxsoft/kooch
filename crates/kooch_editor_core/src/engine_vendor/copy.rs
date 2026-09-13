//! The walk that decides what a vendored engine is made of, and the copy built on it.

use std::fs;
use std::path::Path;

use super::stamp::STAMP_FILE;
use super::{COPY, COPY_ASSETS, VendorError, is_engine_source};

/// Visits every file a vendored engine is made of, as `(path relative to the engine root, absolute
/// path)`.
pub(super) fn walk_engine(
    source: &Path,
    visit: &mut dyn FnMut(&Path, &Path) -> Result<(), VendorError>,
) -> Result<(), VendorError> {
    if !is_engine_source(source) {
        return Err(VendorError::NotAnEngineRoot(source.to_path_buf()));
    }
    for entry in COPY {
        let from = source.join(entry);
        if from.is_dir() {
            walk_dir(&from, Path::new(entry), visit)?;
        } else if from.is_file() {
            visit(Path::new(entry), &from)?;
        }
    }
    for entry in COPY_ASSETS {
        let from = source.join("assets").join(entry);
        if from.is_dir() {
            walk_dir(&from, &Path::new("assets").join(entry), visit)?;
        }
    }
    Ok(())
}

/// Recurses one directory, skipping build output, vcs metadata and test code.
fn walk_dir(
    dir: &Path,
    rel: &Path,
    visit: &mut dyn FnMut(&Path, &Path) -> Result<(), VendorError>,
) -> Result<(), VendorError> {
    let test_mods = cfg_test_modules(dir);
    let mut entries: Vec<_> = fs::read_dir(dir)
        .map_err(VendorError::Io)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(VendorError::Io)?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let name = entry.file_name();
        // The stamp describes the tree, so it cannot be part of what the
        // tree hashes to — that would make the digest depend on itself.
        if name == "target" || name == ".git" || name == STAMP_FILE {
            continue;
        }
        let src = entry.path();
        // 🔴 No test code leaves this repo. A project's copy of the
        // engine is for building a game, and the tests are the engine's
        // own business.
        if is_test_code(&src, &test_mods) {
            continue;
        }
        let child = rel.join(&name);
        if src.is_dir() {
            walk_dir(&src, &child, visit)?;
        } else {
            visit(&child, &src)?;
        }
    }
    Ok(())
}

/// Copies the engine's source into `dest`, which must already exist.
pub(super) fn copy_engine_into(dest: &Path, source: &Path) -> Result<(), VendorError> {
    walk_engine(source, &mut |rel, abs| {
        let to = dest.join(rel);
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(VendorError::Io)?;
        }
        fs::copy(abs, &to).map_err(VendorError::Io)?;
        Ok(())
    })
}

/// `true` for a path that is test code rather than engine code.
pub(super) fn is_test_code(path: &Path, cfg_test_mods: &[String]) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if path.is_dir() {
        return name == "tests" || name == "benches" || cfg_test_mods.iter().any(|m| m == name);
    }
    let stem = name.strip_suffix(".rs").unwrap_or("");
    name == "tests.rs" || name.ends_with("_tests.rs") || cfg_test_mods.iter().any(|m| m == stem)
}

/// Module names this directory's own `.rs` files declare under
/// `#[cfg(test)]`, i.e. the files that exist only for tests.
fn cfg_test_modules(dir: &Path) -> Vec<String> {
    let mut names = Vec::new();

    // 🔴 For `foo/`, the declarations live in `foo.rs` — one level UP, not inside. Rust puts a
    // module's submodules in a directory named after it while the `mod` lines stay in the file.
    let sibling = dir.with_extension("rs");
    if let Ok(text) = fs::read_to_string(&sibling) {
        names.extend(cfg_test_mods_in(&text));
    }

    let Ok(entries) = fs::read_dir(dir) else {
        return names;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        if let Ok(text) = fs::read_to_string(&path) {
            names.extend(cfg_test_mods_in(&text));
        }
    }
    names
}

/// Module names one source file declares under a `test` cfg.
fn cfg_test_mods_in(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut gated = false;
    for line in text.lines() {
        let line = line.trim();
        if gates_on_test(line) {
            gated = true;
            continue;
        }
        if gated && let Some(name) = declared_module(line) {
            names.push(name);
        }
        // Attributes between the gate and the item keep the gate;
        // anything else ends it.
        if !line.starts_with('#') && !line.is_empty() {
            gated = false;
        }
    }
    names
}

/// Whether an attribute compiles its item only for tests.
pub(super) fn gates_on_test(line: &str) -> bool {
    let Some(inner) = line
        .strip_prefix("#[cfg(")
        .and_then(|rest| rest.strip_suffix(")]"))
    else {
        return false;
    };
    // `all(test, …)` only. `any(test, …)` still compiles without tests,
    // and `not(test)` is the opposite claim.
    match inner.strip_prefix("all(").and_then(|r| r.strip_suffix(')')) {
        Some(list) => list.split(',').any(|term| term.trim() == "test"),
        None => inner == "test",
    }
}

/// The name in `mod X;` / `pub(crate) mod X;`, if the line is one.
fn declared_module(line: &str) -> Option<String> {
    let rest = line
        .strip_prefix("pub(crate) ")
        .or_else(|| line.strip_prefix("pub "))
        .unwrap_or(line);
    rest.strip_prefix("mod ")?
        .strip_suffix(';')
        .map(str::to_owned)
}

#[cfg(test)]
mod gate_tests;
