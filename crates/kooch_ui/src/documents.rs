//! Each document's tree from its files (#280): built again when its markup changes on disk,
//! restyled in place when only its sheet does.

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
use crate::templates::expand;
use crate::{UiDocument, UiFocus, UiTree, UiTrees};

/// What a tree was built from, so an unchanged document is not read again.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Origin {
    source: Guid,
    style: Option<Guid>,
    /// The `<use>` files it read, so editing a template rebuilds what uses it.
    templates: Vec<PathBuf>,
    /// Newest mtime of the source and its templates, and the sheet's.
    markup: SystemTime,
    sheet: SystemTime,
}

enum Work {
    Build(UiTree),
    /// `None` when the sheet failed to read: the old look stays until it is fixed.
    Restyle(Option<UiStyleSheet>, SystemTime),
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
    let mut work = Vec::new();
    if let Some(database) = resources.get::<AssetDatabase>() {
        let trees = resources.get::<UiTrees>();
        for (entity, source, style) in documents {
            let previous = trees
                .and_then(|trees| trees.get(entity))
                .and_then(|tree| tree.origin.as_ref())
                .filter(|origin| origin.source == source && origin.style == style);
            // Stat, not read: its files' dates, once a frame per document.
            let sheet = sheet_stamp(database, style);
            match previous {
                Some(origin)
                    if origin.markup == stamp(database, source, None, &origin.templates) =>
                {
                    if origin.sheet == sheet {
                        continue;
                    }
                    // In place, so what the game wrote, hid or focused survives the edit.
                    let read = read_sheet(database, server.as_mut(), style)
                        .inspect_err(|error| tracing::warn!("UI style not reloaded: {error}"))
                        .ok();
                    work.push((entity, Work::Restyle(read, sheet)));
                }
                _ => work.push((
                    entity,
                    Work::Build(build(database, server.as_mut(), source, style)),
                )),
            }
        }
    }
    if let Some(server) = server {
        resources.insert(server);
    }
    let focused = resources.get::<UiFocus>().and_then(|focus| focus.focused);
    let mut refocus = None;
    if let Some(trees) = resources.get_mut::<UiTrees>() {
        for (entity, work) in work {
            match work {
                Work::Build(tree) => {
                    // A rebuilt tree numbers its nodes anew: focus follows the node's name.
                    if let Some((_, node)) = focused.filter(|(own, _)| *own == entity) {
                        refocus = trees
                            .get(entity)
                            .and_then(|old| old.name(node))
                            .and_then(|name| tree.query(name))
                            .map(|node| (entity, node));
                    }
                    trees.insert(entity, tree);
                }
                Work::Restyle(sheet, stamp) => {
                    let Some(tree) = trees.get_mut(entity) else {
                        continue;
                    };
                    if let Some(sheet) = sheet {
                        tree.set_sheet(sheet);
                    }
                    if let Some(origin) = tree.origin.as_mut() {
                        origin.sheet = stamp;
                    }
                }
            }
        }
    }
    if let (Some(node), Some(focus)) = (refocus, resources.get_mut::<UiFocus>()) {
        focus.request(Some(node));
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
    let mut templates = Vec::new();
    let mut tree = match read_tree(database, server, source, style, &mut templates) {
        Ok(tree) => tree,
        Err(error) => {
            tracing::warn!("UI document not built: {error}");
            UiTree::new(taffy::Style::default())
        }
    };
    tree.origin = Some(Origin {
        source,
        style,
        markup: stamp(database, source, None, &templates),
        sheet: sheet_stamp(database, style),
        templates,
    });
    tree
}

fn read_tree(
    database: &AssetDatabase,
    mut server: Option<&mut AssetServer>,
    source: Guid,
    style: Option<Guid>,
    templates: &mut Vec<PathBuf>,
) -> Result<UiTree, String> {
    let source_path = path_of(database, source)?;
    let text = read(server.as_deref_mut(), &source_path)?;
    tree_from(database, server, &text, &source_path, style, templates)
}

/// A tree from markup in memory, its `<use>` paths read from beside `path`: the UI Builder's,
/// mid-edit. Nothing on it is stamped, so [`stale`] never asks for it.
pub fn build_markup(
    database: &AssetDatabase,
    server: Option<&mut AssetServer>,
    text: &str,
    path: &Path,
    style: Option<Guid>,
) -> Result<UiTree, String> {
    tree_from(database, server, text, path, style, &mut Vec::new())
}

fn tree_from(
    database: &AssetDatabase,
    mut server: Option<&mut AssetServer>,
    text: &str,
    source_path: &Path,
    style: Option<Guid>,
    templates: &mut Vec<PathBuf>,
) -> Result<UiTree, String> {
    let markup = UiSource::parse(text).map_err(|e| format!("{}: {e}", source_path.display()))?;
    let markup = expand(
        markup,
        source_path,
        &mut |path| read(server.as_deref_mut(), path),
        templates,
        0,
    )?;
    Ok(build_tree(
        &markup,
        Some(&read_sheet(database, server, style)?),
    ))
}

/// The document's `style` over the controls' sheet, or the engine's default sheet when it has none.
fn read_sheet(
    database: &AssetDatabase,
    server: Option<&mut AssetServer>,
    style: Option<Guid>,
) -> Result<UiStyleSheet, String> {
    let sheet = match style {
        Some(guid) => {
            let path = path_of(database, guid)?;
            let mut sheet = UiStyleSheet::parse(&read(server, &path)?);
            warn_skipped(&path, &sheet);
            let folder = path.parent().unwrap_or(Path::new(""));
            sheet.resolve_images(|url| {
                let guid = database.guid_for(&normalise(&folder.join(url)));
                if guid.is_none() {
                    tracing::warn!("{}: image `{url}` is not a known asset", path.display());
                }
                guid
            });
            sheet
        }
        None => UiStyleSheet::parse(DEFAULT_SHEET),
    };
    // The controls' look first: a tie of specificity goes to the later rule, the document's.
    let mut base = UiStyleSheet::parse(CONTROLS_SHEET);
    base.rules.extend(sheet.rules);
    base.warnings = sheet.warnings;
    Ok(base)
}

const DEFAULT_SHEET: &str = include_str!("default.kcss");
const CONTROLS_SHEET: &str = include_str!("controls.kcss");

/// `a/b/../c` → `a/c`: the database keys paths as scanned, and those have no `..`.
pub(crate) fn normalise(path: &Path) -> PathBuf {
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

pub(crate) fn path_of(database: &AssetDatabase, guid: Guid) -> Result<PathBuf, String> {
    database
        .entry(guid)
        .map(|entry| entry.path.clone())
        .ok_or_else(|| format!("{guid} is not a known asset"))
}

/// Through the server when there is one: a packaged game has nothing on disk.
pub(crate) fn read_bytes(server: Option<&mut AssetServer>, path: &Path) -> Result<Vec<u8>, String> {
    match server {
        Some(server) => server.read_bytes(path).map_err(|e| e.to_string()),
        None => std::fs::read(path).map_err(|e| e.to_string()),
    }
    .map_err(|e| format!("{}: {e}", path.display()))
}

fn read(server: Option<&mut AssetServer>, path: &Path) -> Result<String, String> {
    String::from_utf8(read_bytes(server, path)?).map_err(|e| format!("{}: {e}", path.display()))
}

fn sheet_stamp(database: &AssetDatabase, style: Option<Guid>) -> SystemTime {
    style.map_or(SystemTime::UNIX_EPOCH, |guid| {
        stamp(database, guid, None, &[])
    })
}

/// The newest of the files' mtimes.
pub(crate) fn stamp(
    database: &AssetDatabase,
    source: Guid,
    sheet: Option<Guid>,
    templates: &[PathBuf],
) -> SystemTime {
    [Some(source), sheet]
        .into_iter()
        .flatten()
        .filter_map(|guid| database.entry(guid))
        .map(|entry| entry.path.as_path())
        .chain(templates.iter().map(PathBuf::as_path))
        .filter_map(|path| path.metadata().and_then(|m| m.modified()).ok())
        .max()
        .unwrap_or(SystemTime::UNIX_EPOCH)
}
