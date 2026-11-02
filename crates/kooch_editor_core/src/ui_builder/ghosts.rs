//! What a `<use>` instances, shown under it in the hierarchy as Unity shows a template's
//! contents: the template's elements, which the Inspector edits through `<set node=…>`.

use std::path::Path;

use kooch_ui::markup::Markup;

/// Deeper than this is a template using itself.
const MAX_DEPTH: u32 = 8;

/// An element of an instance's template.
pub(crate) struct Ghost {
    /// Below the `<use>`: 1 is a child of the instance's root.
    pub depth: u32,
    pub tag: String,
    pub name: Option<String>,
    /// As the template writes them, a label's content under `text`.
    pub attributes: Vec<(String, String)>,
    /// The template it is written in.
    pub file: String,
}

/// Each `<use>` of `markup` and its template's elements, as read now.
pub(super) fn of(markup: &Markup, path: &Path) -> Vec<(u32, Vec<Ghost>)> {
    markup
        .outline()
        .into_iter()
        .filter(|row| row.tag == "use")
        .map(|row| (row.origin, instance(markup, row.origin, path, 0)))
        .collect()
}

fn instance(markup: &Markup, origin: u32, at: &Path, depth: u32) -> Vec<Ghost> {
    let mut out = Vec::new();
    let src = markup
        .attributes(origin)
        .into_iter()
        .find(|(key, _)| key == "src");
    let Some((_, src)) = src.filter(|_| depth < MAX_DEPTH) else {
        return out;
    };
    let path = at.parent().unwrap_or(Path::new("")).join(src);
    let Some(template) = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| Markup::parse(&text).ok())
    else {
        return out;
    };
    let file = path
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    // Depth 1 is the instance's root, which is the `<use>` row itself.
    for row in template.outline().into_iter().filter(|row| row.depth >= 2) {
        let mut attributes = template.attributes(row.origin);
        if row.tag == "label" {
            attributes.push(("text".into(), template.text(row.origin).unwrap_or_default()));
        }
        let base = row.depth - 1;
        let nested = (row.tag == "use").then(|| {
            let mut nested = instance(&template, row.origin, &path, depth + 1);
            apply(&template, row.origin, &mut nested);
            nested
        });
        out.push(Ghost {
            depth: base,
            tag: row.tag,
            name: row.name,
            attributes,
            file: file.clone(),
        });
        for mut ghost in nested.into_iter().flatten() {
            ghost.depth += base;
            out.push(ghost);
        }
    }
    out
}

/// A nested `<use>`'s own `<set>`s, on the first node of each name as the runtime lands them.
fn apply(markup: &Markup, origin: u32, ghosts: &mut [Ghost]) {
    for (node, key, value) in markup.overrides(origin) {
        let Some(ghost) = ghosts.iter_mut().find(|g| g.name.as_deref() == Some(&node)) else {
            continue;
        };
        match ghost.attributes.iter_mut().find(|(own, _)| *own == key) {
            Some(slot) => slot.1 = value,
            None => ghost.attributes.push((key, value)),
        }
    }
}
