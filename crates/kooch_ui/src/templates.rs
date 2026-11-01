//! `<use src>` (#280 phase 5): a template's elements spliced in where it is used, so the cascade,
//! the build and focus see one flat document and never a template.

use std::path::{Path, PathBuf};

use crate::documents::normalise;
use crate::source::{Element, ElementKind, NO_PARENT, UiSource};

/// Deeper than this is a template using itself.
const MAX_DEPTH: u8 = 8;

/// `source`, read from `at`, with every `<use>` replaced by its file. Each template read is pushed
/// to `files`, so an edit to one rebuilds the documents using it.
pub(crate) fn expand(
    source: UiSource,
    at: &Path,
    read: &mut dyn FnMut(&Path) -> Result<String, String>,
    files: &mut Vec<PathBuf>,
    depth: u8,
) -> Result<UiSource, String> {
    let mut out: Vec<Element> = Vec::with_capacity(source.elements.len());
    // Where each element of `source` landed in `out`.
    let mut moved: Vec<u32> = Vec::with_capacity(source.elements.len());
    for element in source.elements {
        let parent = match element.parent {
            NO_PARENT => NO_PARENT,
            parent => moved[parent as usize],
        };
        moved.push(out.len() as u32);
        if element.kind != ElementKind::Use {
            out.push(Element { parent, ..element });
            continue;
        }
        if depth >= MAX_DEPTH {
            return Err(format!("{}: templates nest past {MAX_DEPTH}", at.display()));
        }
        let src = attribute(&element, "src")
            .ok_or_else(|| format!("{}: `<use>` without `src`", at.display()))?;
        let path = normalise(&at.parent().unwrap_or(Path::new("")).join(src));
        files.push(path.clone());
        let inner =
            UiSource::parse(&read(&path)?).map_err(|e| format!("{}: {e}", path.display()))?;
        let inner = expand(inner, &path, read, files, depth + 1)?;
        splice(inner, &element, parent, &mut out)
            .map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(UiSource { elements: out })
}

/// The template's one element under `<ui>` becomes the instance root, under `parent`; the
/// `<use>`'s own attributes and `<set>`s land on the copy.
fn splice(
    inner: UiSource,
    with: &Element,
    parent: u32,
    out: &mut Vec<Element>,
) -> Result<(), String> {
    let roots = inner.elements.iter().filter(|e| e.parent == 0).count();
    if roots != 1 {
        return Err("a template's `<ui>` holds one element, the instance".to_owned());
    }
    let base = out.len() as u32;
    // Skipping the `<ui>` moves every index down by one.
    for mut element in inner.elements.into_iter().skip(1) {
        element.origin = with.origin;
        element.parent = match element.parent {
            0 => parent,
            own => base + own - 1,
        };
        out.push(element);
    }
    let root = &mut out[base as usize];
    if with.name.is_some() {
        root.name = with.name.clone();
    }
    root.classes.extend(with.classes.iter().cloned());
    for (key, value) in with.attributes.iter().filter(|(key, _)| &**key != "src") {
        set(root, key, value);
    }
    for (node, key, value) in &with.overrides {
        let target = out[base as usize..]
            .iter_mut()
            .find(|e| e.name.as_deref() == Some(node))
            .ok_or_else(|| format!("`<set node=\"{node}\">` names no node of the template"))?;
        set(target, key, value);
    }
    Ok(())
}

fn set(element: &mut Element, key: &str, value: &str) {
    match key {
        "text" => element.text = Some(value.into()),
        "class" => element.classes = value.split_whitespace().map(Into::into).collect(),
        "name" => element.name = Some(value.into()),
        _ => match element.attributes.iter_mut().find(|(own, _)| &**own == key) {
            Some(slot) => slot.1 = value.into(),
            None => element.attributes.push((key.into(), value.into())),
        },
    }
}

fn attribute<'a>(element: &'a Element, key: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|(own, _)| &**own == key)
        .map(|(_, value)| &**value)
}

/// Two instances of one button, each with its own text, under the right parent.
#[test]
fn a_template_instances_twice() {
    let page = UiSource::parse(
        r#"<ui><panel name="menu">
            <use src="button.kxml" name="play" class="wide"><set node="label" text="Jugar"/></use>
            <use src="button.kxml" name="quit"><set node="label" text="Salir"/></use>
        </panel></ui>"#,
    )
    .unwrap();
    let mut read = |_: &Path| {
        Ok(r#"<ui><button class="button"><label name="label">?</label></button></ui>"#.to_owned())
    };
    let mut files = Vec::new();
    let page = expand(page, Path::new("ui/menu.kxml"), &mut read, &mut files, 0).unwrap();
    let names: Vec<_> = page.elements.iter().map(|e| e.name.as_deref()).collect();
    assert_eq!(
        names,
        [
            None,
            Some("menu"),
            Some("play"),
            Some("label"),
            Some("quit"),
            Some("label")
        ]
    );
    assert_eq!(page.elements[2].parent, 1);
    assert_eq!(page.elements[3].parent, 2);
    assert_eq!(page.elements[5].parent, 4);
    assert_eq!(page.elements[3].text.as_deref(), Some("Jugar"));
    assert_eq!(page.elements[5].text.as_deref(), Some("Salir"));
    assert_eq!(page.elements[2].classes, ["button".into(), "wide".into()]);
    assert_eq!(files, vec![PathBuf::from("ui/button.kxml"); 2]);
}
