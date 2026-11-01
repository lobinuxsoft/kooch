//! `.kxml` (#280, syntax in #1693): a document's structure, read with `roxmltree`.

use kooch_core::asset_loader::{AssetError, AssetLoader, AssetResult, LoadContext};

/// Extension the asset database routes to [`UiSourceLoader`].
pub const UI_EXTENSION: &str = "kxml";

/// The root's parent.
pub const NO_PARENT: u32 = u32::MAX;

/// `on:` and `bind:` are XML namespace prefixes, declared around the file so the `xmlns` line on
/// `<ui>` is optional; a linter still wants it. On line 1, so every row an error names is the file's.
const PREFIXES: [(&str, &str); 2] = [("on", "urn:kooch:on"), ("bind", "urn:kooch:bind")];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementKind {
    Ui,
    Panel,
    Label,
    Button,
    Toggle,
    Slider,
    Use,
}

impl ElementKind {
    pub(crate) const ALL: [Self; 7] = [
        Self::Ui,
        Self::Panel,
        Self::Label,
        Self::Button,
        Self::Toggle,
        Self::Slider,
        Self::Use,
    ];

    pub fn tag(self) -> &'static str {
        match self {
            Self::Ui => "ui",
            Self::Panel => "panel",
            Self::Label => "label",
            Self::Button => "button",
            Self::Toggle => "toggle",
            Self::Slider => "slider",
            Self::Use => "use",
        }
    }
}

/// One element. Parents come before their children.
#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    pub kind: ElementKind,
    /// Index into [`UiSource::elements`]; [`NO_PARENT`] for the root.
    pub parent: u32,
    pub name: Option<Box<str>>,
    pub classes: Vec<Box<str>>,
    /// A label's text, trimmed.
    pub text: Option<Box<str>>,
    /// The rest as written (`on:click`, `bind:value`, `min`, `src`…), for the phases that read them.
    pub attributes: Vec<(Box<str>, Box<str>)>,
    /// A `<use>`'s `<set node attribute=value>` children: node name, attribute, value.
    pub overrides: Vec<(Box<str>, Box<str>, Box<str>)>,
    /// The element of the opened file this one came from, in document order: itself, or the
    /// `<use>` or control that put it here. What the UI Builder selects.
    pub origin: u32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct UiSource {
    pub elements: Vec<Element>,
}

impl UiSource {
    pub fn parse(text: &str) -> Result<Self, String> {
        let body = strip_declaration(text);
        let declared: String = PREFIXES
            .iter()
            .map(|(prefix, uri)| format!(" xmlns:{prefix}=\"{uri}\""))
            .collect();
        let wrapped = format!("<kooch{declared}>{body}</kooch>");
        let document = roxmltree::Document::parse(&wrapped).map_err(|e| e.to_string())?;
        let mut roots = document
            .root_element()
            .children()
            .filter(|n| n.is_element());
        let (Some(root), None) = (roots.next(), roots.next()) else {
            return Err("a .kxml holds one `<ui>`".to_owned());
        };
        if root.tag_name().name() != "ui" {
            return Err(format!(
                "{}: the root is `<ui>`, not `<{}>`",
                at(&document, root),
                root.tag_name().name()
            ));
        }
        if root.attribute("style").is_some() {
            tracing::warn!("`<ui style>` is ignored: the sheet is the UiDocument's Style");
        }
        let mut source = Self {
            elements: Vec::new(),
        };
        push(&document, root, NO_PARENT, &mut source.elements)?;
        Ok(source)
    }
}

/// An `<?xml …?>` line would land inside the wrapper, where XML forbids it.
fn strip_declaration(text: &str) -> &str {
    let trimmed = text.trim_start();
    match trimmed
        .strip_prefix("<?xml")
        .and_then(|rest| rest.find("?>").map(|end| &rest[end + 2..]))
    {
        Some(rest) => rest,
        None => text,
    }
}

fn at(document: &roxmltree::Document<'_>, node: roxmltree::Node<'_, '_>) -> String {
    let position = document.text_pos_at(node.range().start);
    format!("line {}", position.row)
}

fn push(
    document: &roxmltree::Document<'_>,
    node: roxmltree::Node<'_, '_>,
    parent: u32,
    out: &mut Vec<Element>,
) -> Result<(), String> {
    let tag = node.tag_name().name();
    let kind = ElementKind::ALL
        .into_iter()
        .find(|kind| kind.tag() == tag)
        .ok_or_else(|| format!("{}: `<{tag}>` is not an element", at(document, node)))?;
    if parent != NO_PARENT && kind == ElementKind::Ui {
        return Err(format!("{}: `<ui>` is only the root", at(document, node)));
    }
    let mut element = Element {
        kind,
        parent,
        name: None,
        classes: Vec::new(),
        text: None,
        attributes: Vec::new(),
        overrides: Vec::new(),
        origin: out.len() as u32,
    };
    for attribute in node.attributes() {
        let local = attribute.name();
        let prefix = PREFIXES
            .iter()
            .find(|(_, uri)| attribute.namespace() == Some(uri))
            .map(|(prefix, _)| *prefix);
        match (prefix, local) {
            (None, "name") => element.name = Some(attribute.value().into()),
            (None, "class") => {
                element.classes = attribute
                    .value()
                    .split_whitespace()
                    .map(Into::into)
                    .collect();
            }
            (None, "style") if kind == ElementKind::Ui => {}
            (Some(prefix), _) => element
                .attributes
                .push((format!("{prefix}:{local}").into(), attribute.value().into())),
            (None, _) => element
                .attributes
                .push((local.into(), attribute.value().into())),
        }
    }
    if kind == ElementKind::Label {
        let text: String = node
            .children()
            .filter_map(|child| child.text())
            .collect::<String>();
        element.text = Some(text.trim().into());
    }
    // A `<use>`'s children are overrides of the instance, not elements.
    if kind == ElementKind::Use {
        for set in node.children().filter(|n| n.is_element()) {
            let target = set
                .attribute("node")
                .filter(|_| set.tag_name().name() == "set")
                .ok_or_else(|| {
                    format!("{}: `<use>` holds only `<set node=…>`", at(document, set))
                })?;
            for attribute in set.attributes().filter(|a| a.name() != "node") {
                let key = match PREFIXES
                    .iter()
                    .find(|(_, uri)| attribute.namespace() == Some(uri))
                {
                    Some((prefix, _)) => format!("{prefix}:{}", attribute.name()),
                    None => attribute.name().to_owned(),
                };
                element
                    .overrides
                    .push((target.into(), key.into(), attribute.value().into()));
            }
        }
        out.push(element);
        return Ok(());
    }
    let index = out.len() as u32;
    out.push(element);
    for child in node.children().filter(|n| n.is_element()) {
        push(document, child, index, out)?;
    }
    Ok(())
}

/// Reads a `.kxml` file into a [`UiSource`].
#[derive(Debug, Default, Clone, Copy)]
pub struct UiSourceLoader;

impl AssetLoader<UiSource> for UiSourceLoader {
    fn extensions(&self) -> &[&'static str] {
        &[UI_EXTENSION]
    }

    fn load(&self, bytes: &[u8], _ctx: &mut LoadContext<'_>) -> AssetResult<UiSource> {
        let text = std::str::from_utf8(bytes).map_err(|e| AssetError::Loader(Box::new(e)))?;
        UiSource::parse(text).map_err(|e| AssetError::Loader(e.into()))
    }
}

kooch_core::register_asset!(UiSource, UiSourceLoader);
