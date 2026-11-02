//! What the Builder's panels read and change: the markup in memory, written only on save.

use std::path::Path;
use std::time::SystemTime;

use kooch_core::asset_database::AssetDatabase;
use kooch_ui::UiRect;
use kooch_ui::markup::{Markup, Outline, Place};

use super::UiBuilder;
use super::ghosts::Ghost;

/// The selected element, as the Inspector shows it.
pub(crate) struct Inspected {
    pub tag: String,
    /// Everything but `name`, `class` and `style`, which have rows of their own.
    pub attributes: Vec<(String, String)>,
    pub name: String,
    pub classes: String,
    pub style: String,
    /// A label's; `None` for an element with no text.
    pub text: Option<String>,
    /// For a template's node: what the template sets, which an empty field reads.
    pub inherited: Vec<(String, String)>,
    /// The template a node of an instance is written in.
    pub from: Option<String>,
}

impl UiBuilder {
    pub(crate) fn outline(&self) -> Vec<Outline> {
        self.markup.outline()
    }

    pub(crate) fn inspect(&self, origin: u32) -> Option<Inspected> {
        if let Some(node) = self.node_in(origin) {
            return self.inspect_node(origin, node);
        }
        let tag = self.markup.tag(origin)?.to_owned();
        let mut all = self.markup.attributes(origin);
        let mut take = |key: &str| {
            all.iter()
                .position(|(own, _)| own == key)
                .map(|at| all.remove(at).1)
                .unwrap_or_default()
        };
        let (name, classes, style) = (take("name"), take("class"), take("style"));
        let text = (tag == "label").then(|| self.markup.text(origin).unwrap_or_default());
        Some(Inspected {
            tag,
            attributes: all,
            name,
            classes,
            style,
            text,
            inherited: Vec::new(),
            from: None,
        })
    }

    /// A template's node as the instance at `origin` overrides it: its fields are the `<set>`s.
    fn inspect_node(&self, origin: u32, node: &str) -> Option<Inspected> {
        let ghost = self
            .ghosts(origin)
            .iter()
            .find(|ghost| ghost.name.as_deref() == Some(node))?;
        let mut all: Vec<(String, String)> = self
            .markup
            .overrides(origin)
            .into_iter()
            .filter(|(own, _, _)| own == node)
            .map(|(_, key, value)| (key, value))
            .collect();
        let mut take = |key: &str| {
            all.iter()
                .position(|(own, _)| own == key)
                .map(|at| all.remove(at).1)
        };
        let (classes, style) = (take("class"), take("style"));
        let text = take("text");
        let inherited = |key: &str| {
            ghost
                .attributes
                .iter()
                .find(|(own, _)| own == key)
                .map(|(_, value)| value.clone())
        };
        Some(Inspected {
            text: (ghost.tag == "label").then(|| text.unwrap_or_default()),
            tag: ghost.tag.clone(),
            attributes: all,
            name: node.to_owned(),
            // Both replace the template's whole value, so they start from it.
            classes: classes.or_else(|| inherited("class")).unwrap_or_default(),
            style: style.or_else(|| inherited("style")).unwrap_or_default(),
            inherited: ghost.attributes.clone(),
            from: Some(ghost.file.clone()),
        })
    }

    pub(crate) fn ghosts(&self, origin: u32) -> &[Ghost] {
        self.ghosts
            .iter()
            .find(|(own, _)| *own == origin)
            .map_or(&[], |(_, ghosts)| ghosts)
    }

    /// The Inspector edits the selection: with a node of an instance selected, its edits are that
    /// node's `<set>`s on the `<use>`.
    fn node_in(&self, origin: u32) -> Option<&str> {
        self.inner
            .as_deref()
            .filter(|_| self.selected == Some(origin))
    }

    pub(crate) fn select(&mut self, origin: Option<u32>, inner: Option<String>) {
        self.selected = origin;
        self.inner = inner;
    }

    /// An empty value removes the attribute.
    pub(crate) fn set_attribute(&mut self, origin: u32, key: &str, value: &str) {
        let value = (!value.trim().is_empty()).then_some(value);
        let done = match self.node_in(origin).map(str::to_owned) {
            Some(node) => self.markup.set_override(origin, &node, key, value),
            None => self.markup.set_attribute(origin, key, value),
        };
        if done {
            self.changed();
        }
    }

    pub(crate) fn set_text(&mut self, origin: u32, text: &str) {
        if self.node_in(origin).is_some() {
            return self.set_attribute(origin, "text", text);
        }
        self.markup.set_text(origin, text);
        self.changed();
    }

    /// Renames the element itself, never a node its template holds.
    pub(crate) fn rename(&mut self, origin: u32, name: &str) {
        let value = (!name.is_empty()).then_some(name);
        if self.markup.set_attribute(origin, "name", value) {
            self.changed();
        }
    }

    /// Selects what landed. `None` when the markup refused the spot.
    pub(crate) fn insert(&mut self, snippet: &str, at: Place) -> Option<u32> {
        let origin = self.markup.insert(snippet, at)?;
        self.select(Some(origin), None);
        self.changed();
        Some(origin)
    }

    pub(crate) fn move_to(&mut self, origin: u32, at: Place) {
        if let Some(moved) = self.markup.move_to(origin, at) {
            self.select(Some(moved), None);
            self.changed();
        }
    }

    pub(crate) fn duplicate(&mut self, origin: u32) {
        if let Some(copy) = self.markup.duplicate(origin) {
            self.select(Some(copy), None);
            self.changed();
        }
    }

    pub(crate) fn remove(&mut self, origin: u32) {
        if self.markup.remove(origin) {
            self.select(None, None);
            self.changed();
        }
    }

    pub(crate) fn tag(&self, origin: u32) -> Option<&str> {
        self.markup.tag(origin)
    }

    /// Takes child elements, so a drop on it lands inside.
    pub(crate) fn holds_children(&self, origin: u32) -> bool {
        self.markup
            .tag(origin)
            .and_then(kooch_ui::source::ElementKind::from_tag)
            .is_some_and(kooch_ui::source::ElementKind::holds_children)
    }

    /// The element under a point of the canvas, in canvas units.
    pub(crate) fn hit(&self, point: glam::Vec2) -> Option<u32> {
        let tree = self.tree.as_ref()?;
        tree.element_of(tree.hit(point)?)
    }

    /// Where an element landed, in canvas units: a `<use>`'s is its instance's.
    pub(crate) fn rect_of(&self, origin: u32) -> Option<UiRect> {
        let tree = self.tree.as_ref()?;
        tree.rect(tree.node_of(origin)?)
    }

    pub(crate) fn save(&mut self) {
        if let Err(error) = std::fs::write(&self.path, self.markup.write()) {
            tracing::error!(file = %self.path.display(), %error, "could not save the UI document");
            return;
        }
        self.dirty = false;
        // Our own write is not news from disk.
        self.read_at.0 = modified(&self.path);
        tracing::info!(file = %self.path.display(), "UI document saved");
    }

    fn changed(&mut self) {
        self.dirty = true;
        self.edited = true;
    }

    /// A file saved elsewhere replaces the markup, unless it holds edits of ours; a sheet saved
    /// anywhere restyles.
    pub(super) fn reread(&mut self, database: &AssetDatabase) {
        let file = modified(&self.path);
        let sheet = self
            .style
            .and_then(|guid| database.entry(guid))
            .map_or(SystemTime::UNIX_EPOCH, |entry| modified(&entry.path));
        if file != self.read_at.0 && !self.dirty {
            match std::fs::read_to_string(&self.path)
                .map_err(|error| error.to_string())
                .and_then(|text| Markup::parse(&text))
            {
                Ok(markup) => {
                    self.markup = markup;
                    self.edited = true;
                }
                Err(error) => self.error = Some(error),
            }
        }
        if sheet != self.read_at.1 {
            self.edited = true;
        }
        self.read_at = (file, sheet);
    }
}

fn modified(path: &Path) -> SystemTime {
    path.metadata()
        .and_then(|metadata| metadata.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}
