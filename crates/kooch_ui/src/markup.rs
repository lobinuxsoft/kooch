//! An opened `.kxml` as an editable XML tree (#280 phase 8): edited in memory, written on save,
//! with its comments, prefixes and attribute order kept. Indentation is redone on write.

use xmlem::{Document, Element};

/// One row of the UI Builder's hierarchy.
#[derive(Debug, Clone, PartialEq)]
pub struct Outline {
    /// What [`Element::origin`](crate::source::Element::origin) counts.
    pub origin: u32,
    pub depth: u32,
    pub tag: String,
    pub name: Option<String>,
    pub classes: Option<String>,
}

pub struct Markup {
    document: Document,
}

impl Markup {
    pub fn parse(text: &str) -> Result<Self, String> {
        text.parse::<Document>()
            .map(|document| Self { document })
            .map_err(|error| format!("{error:?}"))
    }

    pub fn write(&self) -> String {
        self.document.to_string_pretty()
    }

    pub fn outline(&self) -> Vec<Outline> {
        self.elements()
            .into_iter()
            .enumerate()
            .map(|(origin, (element, depth))| Outline {
                origin: origin as u32,
                depth,
                tag: element.name(&self.document).to_owned(),
                name: element.attribute(&self.document, "name").map(Into::into),
                classes: element.attribute(&self.document, "class").map(Into::into),
            })
            .collect()
    }

    pub fn tag(&self, origin: u32) -> Option<&str> {
        Some(self.element(origin)?.name(&self.document))
    }

    /// As written, prefixes included, in the file's order.
    pub fn attributes(&self, origin: u32) -> Vec<(String, String)> {
        self.element(origin)
            .map(|element| {
                element
                    .attributes(&self.document)
                    .iter()
                    .map(|(key, value)| (key.to_string(), value.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// `None` removes it. A name XML does not allow is refused rather than written.
    pub fn set_attribute(&mut self, origin: u32, key: &str, value: Option<&str>) -> bool {
        let Some(element) = self.element(origin) else {
            return false;
        };
        if !valid_name(key) {
            return false;
        }
        match value {
            Some(value) => element.set_attribute(&mut self.document, key, value),
            None => element.remove_attribute(&mut self.document, key),
        }
        true
    }

    /// A label's content.
    pub fn text(&self, origin: u32) -> Option<String> {
        let element = self.element(origin)?;
        let text: String = element
            .child_nodes(&self.document)
            .iter()
            .filter_map(|node| node.as_text())
            .map(|text| text.as_str(&self.document))
            .collect();
        Some(text.trim().to_owned())
    }

    pub fn set_text(&mut self, origin: u32, text: &str) {
        if let Some(element) = self.element(origin) {
            element.set_text(&mut self.document, text);
        }
    }

    fn element(&self, origin: u32) -> Option<Element> {
        self.elements()
            .get(origin as usize)
            .map(|(element, _)| *element)
    }

    /// In document order, as `UiSource` counts them: a `<set>` belongs to its `<use>`.
    fn elements(&self) -> Vec<(Element, u32)> {
        let mut out = Vec::new();
        walk(&self.document, self.document.root(), 0, &mut out);
        out
    }
}

fn walk(document: &Document, element: Element, depth: u32, out: &mut Vec<(Element, u32)>) {
    if element.name(document) == "set" {
        return;
    }
    out.push((element, depth));
    for child in element.children(document) {
        walk(document, child, depth + 1, out);
    }
}

/// `xmlem` panics on a name XML forbids; one prefix at most, as the files use.
fn valid_name(key: &str) -> bool {
    let part = |part: &str| {
        part.chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    };
    match key.split_once(':') {
        Some((prefix, local)) => part(prefix) && part(local),
        None => part(key),
    }
}

#[test]
fn a_save_keeps_prefixes_and_comments() {
    let text = r#"<?xml version="1.0" encoding="UTF-8"?>
<ui xmlns:on="urn:kooch:on" xmlns:bind="urn:kooch:bind" on:cancel="menu.close">
  <!-- the menu -->
  <panel class="menu">
    <use src="b.kxml" name="play" on:click="menu.play"><set node="label" text="Jugar"/></use>
    <toggle name="hud" bind:value="PauseMenu.show_hud"/>
  </panel>
</ui>"#;
    let mut markup = Markup::parse(text).unwrap();
    assert!(markup.set_attribute(3, "style", Some("width: 10px")));
    let written = markup.write();
    for kept in [
        "on:cancel=\"menu.close\"",
        "<!-- the menu -->",
        "on:click=\"menu.play\"",
        "bind:value=\"PauseMenu.show_hud\"",
        "style=\"width: 10px\"",
        "<set node=\"label\" text=\"Jugar\"",
    ] {
        assert!(written.contains(kept), "{kept} missing from:\n{written}");
    }
    let parsed = crate::source::UiSource::parse(&written).unwrap();
    assert_eq!(parsed.elements.len(), markup.outline().len());
}
