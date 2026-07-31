//! The tree as data: what a folder and a file are, and the walk that
//! builds them from disk.
//!
//! Rebuilt fresh each frame — project trees minus `target/` are small
//! enough that the filesystem walk is trivial.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use kooch_core::Guid;

use crate::actions::NewFileKind;
use crate::panels::inspector::AssetCatalogEntry;

/// A folder node in the tree. Rebuilt fresh each frame (project trees,
/// minus `target/`, are small enough that the fs walk is trivial).
pub(crate) struct FolderNode {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) folders: BTreeMap<String, FolderNode>,
    pub(crate) files: Vec<FileLeaf>,
}

pub(crate) struct FileLeaf {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    /// `Some((guid, type_name))` when the file is a registered typed
    /// asset; `None` for plain files (source, config, …).
    pub(crate) asset: Option<(Guid, String)>,
}

/// In-progress inline rename: which path is being edited + the buffer.
#[derive(Clone)]
pub(crate) struct RenameState {
    pub path: PathBuf,
    pub buffer: String,
    /// Whether the text field has grabbed focus yet (first frame only).
    pub focused: bool,
}

/// In-progress inline creation: the parent folder + what to create. The
/// name is typed before the file is generated, so scripts get the right
/// identifier substituted into their template.
#[derive(Clone)]
pub(crate) struct PendingCreate {
    pub parent: PathBuf,
    pub kind: CreateKind,
    pub buffer: String,
    pub focused: bool,
}

/// What an inline creation produces.
#[derive(Clone, Copy)]
pub(crate) enum CreateKind {
    Folder,
    Material,
    File(NewFileKind),
}

impl FolderNode {
    fn new(name: String, path: PathBuf) -> Self {
        Self {
            name,
            path,
            folders: BTreeMap::new(),
            files: Vec::new(),
        }
    }

    /// Walks `root_path` on disk, overlaying `entries` (the typed asset
    /// catalog) so registered files carry their GUID + type.
    pub(crate) fn build(root_path: &Path, entries: &[&AssetCatalogEntry]) -> Self {
        let by_path: HashMap<&Path, (Guid, &str)> = entries
            .iter()
            .map(|e| (e.path.as_path(), (e.guid, e.type_name.as_str())))
            .collect();
        let mut root = FolderNode::new(String::new(), root_path.to_path_buf());
        walk(&mut root, root_path, &by_path);
        root
    }

    /// `true` when this folder — or any descendant — has a file matching
    /// the (already lowercased) search needle. Empty is always a match.
    pub(crate) fn matches(&self, needle: &str) -> bool {
        if needle.is_empty() {
            return true;
        }
        self.files
            .iter()
            .any(|f| f.name.to_lowercase().contains(needle))
            || self.folders.values().any(|f| f.matches(needle))
    }
}

/// Recursively populates `node` from `dir`. Skips build/vcs noise
/// (`target/`, dot-folders) and `.meta` sidecars (shown via their asset).
pub(super) fn walk(node: &mut FolderNode, dir: &Path, by_path: &HashMap<&Path, (Guid, &str)>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if name == "target" || name.starts_with('.') {
                continue;
            }
            let child = node
                .folders
                .entry(name.clone())
                .or_insert_with(|| FolderNode::new(name, path.clone()));
            walk(child, &path, by_path);
        } else {
            if name.ends_with(".meta") {
                continue;
            }
            let asset = by_path
                .get(path.as_path())
                .map(|(g, t)| (*g, (*t).to_owned()));
            node.files.push(FileLeaf { name, path, asset });
        }
    }
    node.files.sort_by(|a, b| a.name.cmp(&b.name));
}
