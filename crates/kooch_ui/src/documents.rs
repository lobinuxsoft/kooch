//! Each document's tree from its files (#280), built again when they change on disk.

use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use kooch_core::Guid;
use kooch_core::asset_database::AssetDatabase;
use kooch_core::asset_loader::AssetServer;
use kooch_core::resource::Resources;
use kooch_ecs::Entity;
use kooch_ecs::query::Query;

use crate::build::build_tree;
use crate::source::UiSource;
use crate::style::{UiStyleSheet, warn_skipped};
use crate::{UiDocument, UiTree, UiTrees};

/// What a tree was built from, so an unchanged document is not read again.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Origin {
    source: Guid,
    /// The document's own `style`, which overrides the markup's.
    style: Option<Guid>,
    /// The sheet actually read.
    sheet: Option<Guid>,
    modified: SystemTime,
}

/// A document without `source` is left alone: its tree is built in code.
pub fn build_documents(resources: &mut Resources) {
    let mut documents: Vec<(Entity, Guid, Option<Guid>)> = Vec::new();
    Query::<&UiDocument>::new(resources).for_each_entity(|entity, document| {
        if let Some(source) = document.source {
            documents.push((entity, source, document.style));
        }
    });
    if documents.is_empty() {
        return;
    }
    let mut server = resources.remove::<AssetServer>();
    let mut built = Vec::new();
    if let Some(database) = resources.get::<AssetDatabase>() {
        let trees = resources.get::<UiTrees>();
        for (entity, source, style) in documents {
            let previous = trees
                .and_then(|trees| trees.get(entity))
                .and_then(|tree| tree.origin)
                .filter(|origin| origin.source == source && origin.style == style);
            // Stat, not read: two files a frame per document while stopped.
            let sheet = style.or(previous.and_then(|origin| origin.sheet));
            if previous.is_some_and(|origin| origin.modified == stamp(database, source, sheet)) {
                continue;
            }
            built.push((entity, build(database, server.as_mut(), source, style)));
        }
    }
    if let Some(server) = server {
        resources.insert(server);
    }
    if let Some(trees) = resources.get_mut::<UiTrees>() {
        for (entity, tree) in built {
            trees.insert(entity, tree);
        }
    }
}

/// A file that fails still gets a tree, empty, stamped: it is read again when it changes, not
/// every frame.
fn build(
    database: &AssetDatabase,
    server: Option<&mut AssetServer>,
    source: Guid,
    style: Option<Guid>,
) -> UiTree {
    let mut sheet_guid = style;
    let mut tree = match read_tree(database, server, source, &mut sheet_guid) {
        Ok(tree) => tree,
        Err(error) => {
            tracing::warn!("UI document not built: {error}");
            UiTree::new(taffy::Style::default())
        }
    };
    tree.origin = Some(Origin {
        source,
        style,
        sheet: sheet_guid,
        modified: stamp(database, source, sheet_guid),
    });
    tree
}

fn read_tree(
    database: &AssetDatabase,
    mut server: Option<&mut AssetServer>,
    source: Guid,
    sheet_guid: &mut Option<Guid>,
) -> Result<UiTree, String> {
    let source_path = path_of(database, source)?;
    let markup = UiSource::parse(&read(server.as_deref_mut(), &source_path)?)
        .map_err(|e| format!("{}: {e}", source_path.display()))?;
    if sheet_guid.is_none()
        && let Some(relative) = markup.style.as_deref()
    {
        let path = normalise(&source_path.parent().unwrap_or(Path::new("")).join(relative));
        *sheet_guid = database.guid_for(&path);
        if sheet_guid.is_none() {
            tracing::warn!(
                "{}: style `{relative}` is not a known asset",
                source_path.display()
            );
        }
    }
    let sheet = match *sheet_guid {
        Some(guid) => {
            let path = path_of(database, guid)?;
            let sheet = UiStyleSheet::parse(&read(server, &path)?);
            warn_skipped(&path, &sheet);
            Some(sheet)
        }
        None => None,
    };
    Ok(build_tree(&markup, sheet.as_ref()))
}

fn path_of(database: &AssetDatabase, guid: Guid) -> Result<PathBuf, String> {
    database
        .entry(guid)
        .map(|entry| entry.path.clone())
        .ok_or_else(|| format!("{guid} is not a known asset"))
}

/// Through the server when there is one: a packaged game has nothing on disk.
fn read(server: Option<&mut AssetServer>, path: &Path) -> Result<String, String> {
    let bytes = match server {
        Some(server) => server.read_bytes(path).map_err(|e| e.to_string()),
        None => std::fs::read(path).map_err(|e| e.to_string()),
    }
    .map_err(|e| format!("{}: {e}", path.display()))?;
    String::from_utf8(bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// The newest of the files' mtimes.
fn stamp(database: &AssetDatabase, source: Guid, sheet: Option<Guid>) -> SystemTime {
    [Some(source), sheet]
        .into_iter()
        .flatten()
        .filter_map(|guid| database.entry(guid))
        .filter_map(|entry| entry.path.metadata().and_then(|m| m.modified()).ok())
        .max()
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

/// `a/b/../c` → `a/c`: the database keys paths as scanned, and those have no `..`.
fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}
