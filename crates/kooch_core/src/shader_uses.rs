//! `// use: <path>`: a shader calling functions that live in another file (#1807).
//!
//! WGSL has no `#include`, and a module's declarations can come in any order, so the files a shader
//! uses are appended after its own text: its line numbers stay the ones its author sees.

use std::fmt::Display;
use std::path::{Component, Path, PathBuf};

/// What the block comments an editor stores in a file start with: a graph, its notes.
const EDITOR_BLOCK: &str = "/*KOOCH_";

/// What names a file of the engine's own library rather than a path.
pub const ENGINE_PREFIX: &str = "engine:";

/// The engine's functions. In the binary, so a project reaches them with no file of its own to
/// vendor, package or keep up to date.
pub const LIBRARY: [(&str, &str); 2] = [
    ("engine:patterns", include_str!("shader_uses/patterns.wgsl")),
    ("engine:shapes", include_str!("shader_uses/shapes.wgsl")),
];

/// The text of the library file `name`.
pub fn engine_source(name: &str) -> Option<&'static str> {
    LIBRARY
        .iter()
        .find(|(file, _)| *file == name)
        .map(|(_, text)| *text)
}

/// Every file `path` reaches through `// use:`, with its text, each once. A cycle ends here.
pub fn used_files<E: Display>(
    path: &Path,
    source: &str,
    read: &mut dyn FnMut(&Path) -> Result<Vec<u8>, E>,
) -> Result<Vec<(PathBuf, String)>, String> {
    let root = collapsed(path);
    let mut found: Vec<(PathBuf, String)> = Vec::new();
    let mut pending = named(&root, source)?;
    while let Some(file) = pending.pop() {
        if file == root || found.iter().any(|(seen, _)| *seen == file) {
            continue;
        }
        let name = file.to_string_lossy();
        let text = if name.starts_with(ENGINE_PREFIX) {
            engine_source(&name)
                .ok_or_else(|| format!("// use: the engine has no `{name}`"))?
                .to_owned()
        } else {
            let bytes = read(&file).map_err(|e| format!("// use: {name}: {e}"))?;
            String::from_utf8(bytes).map_err(|e| format!("// use: {name}: {e}"))?
        };
        pending.extend(named(&file, &text)?);
        found.push((file, text));
    }
    Ok(found)
}

/// `source` followed by the code of what it uses.
pub fn flatten(source: &str, used: &[(PathBuf, String)]) -> String {
    let mut flat = source.to_owned();
    for (_, text) in used {
        flat.push('\n');
        flat.push_str(&code_of(text));
    }
    flat
}

/// The files `source` names in its leading comments, where `// kind:` lives too.
fn named(path: &Path, source: &str) -> Result<Vec<PathBuf>, String> {
    let folder = path.parent().unwrap_or(Path::new(""));
    source
        .lines()
        .map(str::trim)
        .take_while(|line| line.is_empty() || line.starts_with("//"))
        .filter_map(|line| line.strip_prefix("//")?.trim().strip_prefix("use:"))
        .map(|name| {
            let name = Path::new(name.trim());
            if name.to_string_lossy().starts_with(ENGINE_PREFIX) {
                return Ok(name.to_path_buf());
            }
            if name.is_absolute() {
                return Err(format!("// use: {} is not relative", name.display()));
            }
            Ok(collapsed(&folder.join(name)))
        })
        .collect()
}

/// `path` with its `..` resolved, the one spelling two shaders naming a file agree on. A leading
/// `.` stays: a pack matches against its root as it was mounted.
fn collapsed(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        let climbs = part == Component::ParentDir;
        if climbs && matches!(out.components().next_back(), Some(Component::Normal(_))) {
            out.pop();
        } else {
            out.push(part);
        }
    }
    out
}

/// `text` without its editor blocks, which can be most of a graph's file.
fn code_of(text: &str) -> String {
    let mut code = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(EDITOR_BLOCK) {
        code.push_str(&rest[..start]);
        let Some(end) = rest[start..].find("*/") else {
            return code;
        };
        rest = &rest[start + end + 2..];
    }
    code.push_str(rest);
    code
}

#[cfg(test)]
mod tests;
