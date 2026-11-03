//! An opened `.kxml` as an editable XML tree (#280 phase 8): edited in memory, written on save,
//! with its comments, prefixes and attribute order kept. Indentation is redone on write.

use xmlem::{Document, Element, NewElement, Node};

/// One row of the UI Builder's hierarchy.
#[derive(Debug, Clone, PartialEq)]
pub struct Outline {
    /// What [`Element::origin`](crate::source::Element::origin) counts.
    pub origin: u32,
    pub depth: u32,
    pub tag: String,
    pub name: Option<String>,
    pub classes: Option<String>,
    /// The author's note, `doc=""`.
    pub doc: Option<String>,
}

/// Where an element lands, by the origin of the one it goes into or beside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// Its last child.
    Inside(u32),
    Before(u32),
    After(u32),
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
                doc: element.attribute(&self.document, "doc").map(Into::into),
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

    /// A `<use>`'s `<set>`s, one `(node, attribute, value)` each, in the file's order.
    pub fn overrides(&self, origin: u32) -> Vec<(String, String, String)> {
        let Some(element) = self.element(origin) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for set in self.sets(element) {
            let Some(node) = set.attribute(&self.document, "node") else {
                continue;
            };
            for (key, value) in set.attributes(&self.document) {
                if key.to_string() != "node" {
                    out.push((node.to_owned(), key.to_string(), value.clone()));
                }
            }
        }
        out
    }

    /// One attribute of a template's `node`, overridden on the `<use>` at `origin`. `None` removes
    /// it, and a `<set>` left with nothing to set goes too.
    pub fn set_override(
        &mut self,
        origin: u32,
        node: &str,
        key: &str,
        value: Option<&str>,
    ) -> bool {
        let Some(element) = self.element(origin) else {
            return false;
        };
        if !valid_name(key) || key == "node" {
            return false;
        }
        let of_node: Vec<Element> = self
            .sets(element)
            .into_iter()
            .filter(|set| set.attribute(&self.document, "node") == Some(node))
            .collect();
        let holding = of_node
            .iter()
            .copied()
            .find(|set| set.attribute(&self.document, key).is_some());
        match (value, holding) {
            (Some(value), holding) => {
                let set = match holding.or(of_node.first().copied()) {
                    Some(set) => set,
                    None => {
                        let Ok(parsed) = "<set/>".parse::<Document>() else {
                            return false;
                        };
                        let owned = snapshot(&parsed, parsed.root());
                        let Some(set) = self.place(owned, Spot::Inside(element)) else {
                            return false;
                        };
                        set.set_attribute(&mut self.document, "node", node);
                        set
                    }
                };
                set.set_attribute(&mut self.document, key, value);
            }
            (None, Some(set)) => {
                set.remove_attribute(&mut self.document, key);
                if set.attributes(&self.document).len() <= 1 {
                    element.remove_child(&mut self.document, Node::Element(set));
                }
            }
            (None, None) => {}
        }
        true
    }

    fn sets(&self, element: Element) -> Vec<Element> {
        element
            .children(&self.document)
            .into_iter()
            .filter(|child| child.name(&self.document) == "set")
            .collect()
    }

    /// Parses `snippet`, one element and its children, into `at`. Returns its origin.
    pub fn insert(&mut self, snippet: &str, at: Place) -> Option<u32> {
        let parsed = snippet.parse::<Document>().ok()?;
        let owned = snapshot(&parsed, parsed.root());
        let spot = self.spot(at)?;
        let element = self.place(owned, spot)?;
        self.origin_of(element)
    }

    /// The root stays: a file has one.
    pub fn remove(&mut self, origin: u32) -> bool {
        let Some(element) = self.element(origin) else {
            return false;
        };
        let Some(parent) = element.parent(&self.document) else {
            return false;
        };
        parent.remove_child(&mut self.document, Node::Element(element));
        true
    }

    /// A copy right after it. Returns the copy's origin.
    pub fn duplicate(&mut self, origin: u32) -> Option<u32> {
        let element = self.element(origin)?;
        element.parent(&self.document)?;
        let owned = snapshot(&self.document, element);
        let copy = self.place(owned, Spot::After(element))?;
        self.origin_of(copy)
    }

    /// Refused into itself or its own subtree. Returns its new origin.
    pub fn move_to(&mut self, origin: u32, at: Place) -> Option<u32> {
        let element = self.element(origin)?;
        let parent = element.parent(&self.document)?;
        let spot = self.spot(at)?;
        let anchor = match spot {
            Spot::Inside(anchor) | Spot::Before(anchor) | Spot::After(anchor) => anchor,
        };
        if self.within(anchor, element) {
            return (anchor == element && !matches!(spot, Spot::Inside(_))).then_some(origin);
        }
        let owned = snapshot(&self.document, element);
        parent.remove_child(&mut self.document, Node::Element(element));
        let moved = self.place(owned, spot)?;
        self.origin_of(moved)
    }

    /// `element` is `ancestor` or inside it.
    fn within(&self, element: Element, ancestor: Element) -> bool {
        let mut at = Some(element);
        while let Some(current) = at {
            if current == ancestor {
                return true;
            }
            at = current.parent(&self.document);
        }
        false
    }

    /// Resolved before anything moves: an edit shifts every origin after it.
    fn spot(&self, at: Place) -> Option<Spot> {
        let spot = match at {
            Place::Inside(origin) => Spot::Inside(self.element(origin)?),
            Place::Before(origin) => Spot::Before(self.element(origin)?),
            Place::After(origin) => Spot::After(self.element(origin)?),
        };
        match spot {
            Spot::Before(anchor) | Spot::After(anchor) => {
                anchor.parent(&self.document)?;
            }
            Spot::Inside(_) => {}
        }
        Some(spot)
    }

    fn place(&mut self, owned: Owned, spot: Spot) -> Option<Element> {
        let Owned::Element(new, children) = owned else {
            return None;
        };
        let document = &mut self.document;
        let element = match spot {
            Spot::Inside(parent) => parent.append_new_element(document, new),
            Spot::After(sibling) => sibling.append_new_element_after(document, new),
            Spot::Before(sibling) => match sibling.prev_sibling_element(document) {
                Some(previous) => previous.append_new_element_after(document, new),
                // `xmlem` only inserts after: land after the first child, then move it behind.
                None => {
                    let element = sibling.append_new_element_after(document, new);
                    let first = snapshot(document, sibling);
                    let parent = sibling.parent(document)?;
                    parent.remove_child(document, Node::Element(sibling));
                    if let Owned::Element(first, grand) = first {
                        let moved = element.append_new_element_after(document, first);
                        fill(document, moved, grand);
                    }
                    element
                }
            },
        };
        fill(&mut self.document, element, children);
        Some(element)
    }

    fn origin_of(&self, element: Element) -> Option<u32> {
        self.elements()
            .iter()
            .position(|(own, _)| *own == element)
            .map(|at| at as u32)
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

#[derive(Clone, Copy)]
enum Spot {
    Inside(Element),
    Before(Element),
    After(Element),
}

/// A subtree held apart from any document, to be written back elsewhere.
enum Owned {
    Element(NewElement, Vec<Owned>),
    Text(String),
    CData(String),
    Comment(String),
}

fn snapshot(document: &Document, element: Element) -> Owned {
    let new = NewElement {
        name: element.qname(document).clone(),
        attrs: element.attributes(document).clone(),
    };
    let children = element
        .child_nodes(document)
        .iter()
        .filter_map(|node| match *node {
            Node::Element(child) => Some(snapshot(document, child)),
            Node::Text(text) => Some(Owned::Text(text.as_str(document).to_owned())),
            Node::CDataSection(data) => Some(Owned::CData(data.as_str(document).to_owned())),
            Node::Comment(comment) => Some(Owned::Comment(comment.as_str(document).to_owned())),
            _ => None,
        })
        .collect();
    Owned::Element(new, children)
}

fn fill(document: &mut Document, parent: Element, children: Vec<Owned>) {
    for child in children {
        match child {
            Owned::Element(new, grand) => {
                let element = parent.append_new_element(document, new);
                fill(document, element, grand);
            }
            Owned::Text(text) => {
                parent.append_text(document, &text);
            }
            Owned::CData(data) => {
                parent.append_cdata(document, &data);
            }
            Owned::Comment(comment) => {
                parent.append_comment(document, &comment);
            }
        }
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

#[test]
fn edits_keep_the_order() {
    let text = "<ui><panel name=\"a\"><label>A</label></panel><panel name=\"b\"/></ui>";
    let mut markup = Markup::parse(text).unwrap();
    let names = |markup: &Markup| -> Vec<String> {
        markup
            .outline()
            .iter()
            .map(|row| row.name.clone().unwrap_or(row.tag.clone()))
            .collect()
    };
    assert_eq!(markup.move_to(3, Place::Before(1)), Some(1));
    assert_eq!(names(&markup), ["ui", "b", "a", "label"]);
    assert_eq!(markup.move_to(2, Place::Inside(2)), None);
    assert_eq!(
        markup.insert("<button><label>Go</label></button>", Place::After(1)),
        Some(2)
    );
    assert_eq!(names(&markup), ["ui", "b", "button", "label", "a", "label"]);
    assert_eq!(markup.duplicate(4), Some(6));
    assert!(markup.remove(2));
    assert_eq!(names(&markup), ["ui", "b", "a", "label", "a", "label"]);
    assert!(markup.write().contains("<label>A</label>"));
}
