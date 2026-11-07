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

use crate::build::{build_tree, disabled};
use crate::cascade::resolve;
use crate::source::UiSource;
use crate::style::{ResolvedStyle, UiStyleSheet, warn_skipped};
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

/// One tree kept in step with its files outside the ECS: an editor panel's (#1791). Built when
/// missing or when its markup or a template changed, restyled in place when only the sheet did.
pub fn refresh_tree(
    database: &AssetDatabase,
    mut server: Option<&mut AssetServer>,
    tree: &mut Option<UiTree>,
    source: Guid,
    style: Option<Guid>,
) -> Refreshed {
    let sheet = sheet_stamp(database, style);
    let current = tree.as_mut().filter(|tree| {
        tree.origin
            .as_ref()
            .is_some_and(|origin| origin.source == source && origin.style == style)
    });
    if let Some(current) = current {
        let origin = current.origin.as_ref().expect("filtered on its origin");
        if origin.markup == stamp(database, source, None, &origin.templates) {
            if origin.sheet == sheet {
                return Refreshed::Kept;
            }
            match read_sheet(database, server.as_deref_mut(), style) {
                Ok(read) => current.set_sheet(read),
                Err(error) => tracing::warn!("UI style not reloaded: {error}"),
            }
            if let Some(origin) = current.origin.as_mut() {
                origin.sheet = sheet;
            }
            return Refreshed::Restyled;
        }
    }
    *tree = Some(build(database, server, source, style));
    Refreshed::Rebuilt
}

/// What [`refresh_tree`] did. A rebuilt tree numbers its nodes anew: a `UiNode` held from before
/// is stale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refreshed {
    Kept,
    Restyled,
    Rebuilt,
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
    let engine = engine_assets(server.as_deref());
    let markup = expand(
        markup,
        source_path,
        &mut |path| read(server.as_deref_mut(), path),
        templates,
        &|path| image_guid(database, engine.as_deref(), path),
        0,
    )?;
    Ok(build_tree(
        &markup,
        Some(&read_sheet(database, server, style)?),
    ))
}

/// Instances `markup` — one `<use src="…">` with its attributes and `<set>`s, written as a
/// `.kxml` writes it — as the last child of `parent` in `document`'s tree (#1784). `src` reads
/// beside the document's file, in the editor and in a build. Returns the instance's root.
///
/// A rebuild from the file (an edit to it in the editor) drops instances: fill again when a node
/// you instanced is gone.
pub fn ui_instance(
    resources: &mut Resources,
    document: Entity,
    parent: crate::UiNode,
    markup: &str,
) -> Result<crate::UiNode, String> {
    let source = resources
        .get::<UiTrees>()
        .and_then(|trees| trees.get(document))
        .and_then(|tree| tree.origin.as_ref())
        .map(|origin| origin.source)
        .ok_or("the document was not built from a file")?;
    let mut server = resources.remove::<AssetServer>();
    let expanded = match resources.get::<AssetDatabase>() {
        Some(database) => instance_source(database, server.as_mut(), source, markup),
        None => Err("no asset database".to_owned()),
    };
    if let Some(server) = server {
        resources.insert(server);
    }
    let instance = expanded?;
    let tree = resources
        .get_mut::<UiTrees>()
        .and_then(|trees| trees.get_mut(document))
        .ok_or("the document has no tree")?;
    crate::build::graft(tree, parent, &instance)
        .ok_or_else(|| "the parent is not a node of the document".to_owned())
}

fn instance_source(
    database: &AssetDatabase,
    mut server: Option<&mut AssetServer>,
    source: Guid,
    markup: &str,
) -> Result<UiSource, String> {
    let at = path_of(database, source)?;
    let parsed = UiSource::parse(&format!("<ui>{markup}</ui>"))?;
    let engine = engine_assets(server.as_deref());
    expand(
        parsed,
        &at,
        &mut |path| read(server.as_deref_mut(), path),
        &mut Vec::new(),
        &|path| image_guid(database, engine.as_deref(), path),
        0,
    )
}

/// What each element of `source` ends up with at rest, for showing it: a document's sheet decides
/// more than its own text does. `align-self` left unset reads as the parent's `align-items`, which
/// is what it does. Empty when the sheet cannot be read.
pub fn styles_of(
    database: &AssetDatabase,
    server: Option<&mut AssetServer>,
    source: &UiSource,
    style: Option<Guid>,
) -> Vec<ResolvedStyle> {
    let Ok(sheet) = read_sheet(database, server, style) else {
        return Vec::new();
    };
    let states: Vec<u8> = source.elements.iter().map(disabled).collect();
    let mut styles = resolve(source, Some(&sheet), &states);
    for (index, element) in source.elements.iter().enumerate() {
        let parent = styles
            .get(element.parent as usize)
            .map(|s| s.layout.align_items);
        if let (Some(items), Some(own)) = (parent, styles.get_mut(index))
            && own.layout.align_self.is_none()
        {
            own.layout.align_self = Some(items);
        }
    }
    styles
}

/// The document's `style` over the controls' sheet, or the engine's default sheet when it has none.
fn read_sheet(
    database: &AssetDatabase,
    server: Option<&mut AssetServer>,
    style: Option<Guid>,
) -> Result<UiStyleSheet, String> {
    let engine = engine_assets(server.as_deref());
    let sheet = match style {
        Some(guid) => {
            let path = path_of(database, guid)?;
            let mut sheet = UiStyleSheet::parse(&read(server, &path)?);
            warn_skipped(&path, &sheet);
            let folder = path.parent().unwrap_or(Path::new(""));
            sheet.resolve_images(|url| {
                let guid = image_guid(database, engine.as_deref(), &normalise(&folder.join(url)));
                if guid.is_none() {
                    tracing::warn!("{}: image `{url}` is not a known asset", path.display());
                }
                guid
            });
            sheet
        }
        None => UiStyleSheet::parse(DEFAULT_SHEET),
    };
    Ok(over_controls(sheet))
}

/// The controls' look first: a tie of specificity goes to the later rule, the document's.
pub(crate) fn over_controls(sheet: UiStyleSheet) -> UiStyleSheet {
    let mut base = UiStyleSheet::parse(CONTROLS_SHEET);
    base.rules.extend(sheet.rules);
    base.warnings = sheet.warnings;
    base
}

pub(crate) const DEFAULT_SHEET: &str = include_str!("default.kcss");
const CONTROLS_SHEET: &str = include_str!("controls.kcss");

/// The engine's `assets/` while it is a tree of its own (the editor, Play); a build merges it into
/// the project's, so there the path resolves as written.
fn engine_assets(server: Option<&AssetServer>) -> Option<PathBuf> {
    server?.asset_root().map(Path::to_path_buf)
}

/// The asset at `path`, or at its name under the engine's `assets/`: the editor writes an engine
/// image where a build puts it, inside the project's.
fn image_guid(database: &AssetDatabase, engine: Option<&Path>, path: &Path) -> Option<Guid> {
    database.guid_for(path).or_else(|| {
        let engine = engine?;
        let parts: Vec<Component> = path.components().collect();
        parts
            .iter()
            .enumerate()
            .filter(|(_, part)| part.as_os_str() == "assets")
            .find_map(|(at, _)| {
                let name: PathBuf = parts[at + 1..].iter().collect();
                database.guid_for(&engine.join(name))
            })
    })
}

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
