//! What the Builder's panels read and change: the markup in memory, written only on save.

use std::path::Path;
use std::time::SystemTime;

use kooch_core::asset_database::AssetDatabase;
use kooch_ui::UiRect;
use kooch_ui::markup::{Markup, Outline};

use super::UiBuilder;

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
}

impl UiBuilder {
    pub(crate) fn outline(&self) -> Vec<Outline> {
        self.markup.outline()
    }

    pub(crate) fn inspect(&self, origin: u32) -> Option<Inspected> {
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
        })
    }

    /// An empty value removes the attribute.
    pub(crate) fn set_attribute(&mut self, origin: u32, key: &str, value: &str) {
        let value = (!value.trim().is_empty()).then_some(value);
        if self.markup.set_attribute(origin, key, value) {
            self.changed();
        }
    }

    pub(crate) fn set_text(&mut self, origin: u32, text: &str) {
        self.markup.set_text(origin, text);
        self.changed();
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
