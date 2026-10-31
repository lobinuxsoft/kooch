//! Each document's tree from its files (#280), built again when they change on disk.

use std::path::{Path, PathBuf};
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
    style: Option<Guid>,
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
            if previous.is_some_and(|origin| origin.modified == stamp(database, source, style)) {
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
    let mut tree = match read_tree(database, server, source, style) {
        Ok(tree) => tree,
        Err(error) => {
            tracing::warn!("UI document not built: {error}");
            UiTree::new(taffy::Style::default())
        }
    };
    tree.origin = Some(Origin {
        source,
        style,
        modified: stamp(database, source, style),
    });
    tree
}

/// The document's `style`, or the engine's default sheet when it has none.
fn read_tree(
    database: &AssetDatabase,
    mut server: Option<&mut AssetServer>,
    source: Guid,
    style: Option<Guid>,
) -> Result<UiTree, String> {
    let source_path = path_of(database, source)?;
    let markup = UiSource::parse(&read(server.as_deref_mut(), &source_path)?)
        .map_err(|e| format!("{}: {e}", source_path.display()))?;
    let sheet = match style {
        Some(guid) => {
            let path = path_of(database, guid)?;
            let sheet = UiStyleSheet::parse(&read(server, &path)?);
            warn_skipped(&path, &sheet);
            sheet
        }
        None => UiStyleSheet::parse(DEFAULT_SHEET),
    };
    Ok(build_tree(&markup, Some(&sheet)))
}

const DEFAULT_SHEET: &str = include_str!("default.kcss");

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
