//! What the Inspector suggests: the open sheet's classes and rules, and every action the project's
//! `.kxml` files send or its Rust code reads with `ui_actions`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use kooch_core::Guid;
use kooch_core::asset_database::AssetDatabase;
use kooch_ui::markup::Markup;
use kooch_ui::source::UiSource;
use kooch_ui::style::UiStyleSheet;

use super::{SOURCE_TYPE, UiBuilder};
use crate::panels::inspector::AssetCatalogEntry;

/// How stale the action list may get while the Inspector shows it.
const RESCAN: Duration = Duration::from_secs(2);

pub(crate) struct OpenSheet {
    pub name: String,
    pub text: String,
    pub parsed: UiStyleSheet,
}

/// An action name and where it shows up.
pub(crate) struct KnownAction {
    pub name: String,
    /// Some `.kxml` sends it.
    pub sent: bool,
    /// Some system reads it with `ui_actions`.
    pub read: bool,
}

#[derive(Default)]
pub(crate) struct Known {
    pub sheet: Option<OpenSheet>,
    /// The file as written, templates not spliced: element `i` is markup origin `i`.
    pub file: Option<UiSource>,
    actions: Vec<KnownAction>,
    scanned: Option<Instant>,
}

impl Known {
    pub(super) fn refresh(&mut self, database: &AssetDatabase, text: &str, style: Option<Guid>) {
        self.file = UiSource::parse(text).ok();
        self.sheet = style
            .and_then(|guid| database.entry(guid))
            .and_then(|entry| {
                let text = std::fs::read_to_string(&entry.path).ok()?;
                Some(OpenSheet {
                    name: file_name(&entry.path),
                    parsed: UiStyleSheet::parse(&text),
                    text,
                })
            });
    }
}

impl UiBuilder {
    pub(crate) fn actions(&mut self, catalog: &[AssetCatalogEntry]) -> &[KnownAction] {
        if self.known.scanned.is_none_or(|at| at.elapsed() > RESCAN) {
            self.known.actions = scan(&self.path, &self.markup, catalog);
            self.known.scanned = Some(Instant::now());
        }
        &self.known.actions
    }
}

fn scan(path: &Path, open: &Markup, catalog: &[AssetCatalogEntry]) -> Vec<KnownAction> {
    let mut sent = sent_by(open);
    for entry in catalog
        .iter()
        .filter(|e| e.type_name == SOURCE_TYPE && e.path != path)
    {
        if let Some(markup) = std::fs::read_to_string(&entry.path)
            .ok()
            .and_then(|text| Markup::parse(&text).ok())
        {
            sent.extend(sent_by(&markup));
        }
    }
    let mut read = Vec::new();
    if let Some(root) = path
        .ancestors()
        .find(|dir| dir.join("Cargo.toml").is_file())
    {
        for file in rust_files(&root.join("src")) {
            if let Ok(text) = std::fs::read_to_string(file) {
                read.extend(read_by(&text));
            }
        }
    }
    let mut names: Vec<&String> = sent.iter().chain(&read).collect();
    names.sort_unstable();
    names.dedup();
    names
        .into_iter()
        .map(|name| KnownAction {
            name: name.clone(),
            sent: sent.contains(name),
            read: read.contains(name),
        })
        .collect()
}

fn sent_by(markup: &Markup) -> Vec<String> {
    (0..markup.outline().len() as u32)
        .flat_map(|origin| markup.attributes(origin))
        .filter(|(key, value)| key.starts_with("on:") && !value.is_empty())
        .map(|(_, value)| value)
        .collect()
}

/// The string literal each `ui_actions(…, "name")` names.
fn read_by(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (at, _) in text.match_indices("ui_actions(") {
        let call = &text[at..];
        let Some(end) = call.find(')') else {
            continue;
        };
        let args = &call[..end];
        let Some(open) = args.find('"') else {
            continue;
        };
        if let Some(close) = args[open + 1..].find('"') {
            out.push(args[open + 1..open + 1 + close].to_owned());
        }
    }
    out
}

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(rust_files(&path));
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
    out
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}
