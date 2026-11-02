//! Which rules reach which element, and in what order.

use taffy::Size;
use taffy::prelude::percent;

use crate::source::{Element, NO_PARENT, UiSource};
use crate::style::{Compound, ResolvedStyle, STATE_ROOT, Selector, UiStyleSheet};

/// CSS's: names, then classes and states, then elements.
type Specificity = (u32, u32, u32);

/// One style per element, in `source.elements` order. `states` holds each element's `STATE_*`.
pub fn resolve(
    source: &UiSource,
    sheet: Option<&UiStyleSheet>,
    states: &[u8],
) -> Vec<ResolvedStyle> {
    let mut resolved: Vec<ResolvedStyle> = Vec::with_capacity(source.elements.len());
    let mut matched: Vec<(Specificity, usize)> = Vec::new();
    for (index, element) in source.elements.iter().enumerate() {
        let mut style = match element.parent {
            NO_PARENT => root(),
            parent => inherited(&resolved[parent as usize]),
        };
        if let Some(sheet) = sheet {
            matched.clear();
            for (rule, entry) in sheet.rules.iter().enumerate() {
                let best = entry
                    .selectors
                    .iter()
                    .filter(|selector| matches(selector, &source.elements, states, index))
                    .map(specificity)
                    .max();
                if let Some(best) = best {
                    matched.push((best, rule));
                }
            }
            // The rule index breaks ties: the later rule wins, as in CSS.
            matched.sort_unstable();
            for &(_, rule) in &matched {
                for declaration in &sheet.rules[rule].declarations {
                    style.apply(declaration);
                }
            }
        }
        // Inline last: it beats any rule, as in CSS.
        if let Some((_, inline)) = element.attributes.iter().find(|(key, _)| &**key == "style") {
            for declaration in UiStyleSheet::parse_inline(inline) {
                style.apply(&declaration);
            }
        }
        resolved.push(style);
    }
    resolved
}

/// The rules that reach element `index` in any state, weakest first, as the cascade applies them.
pub fn matching_rules(source: &UiSource, sheet: &UiStyleSheet, index: usize) -> Vec<usize> {
    // Every state on, so a `:hover` rule shows too; `:root` stays the root's alone.
    let states = vec![!STATE_ROOT; source.elements.len()];
    let mut matched: Vec<(Specificity, usize)> = sheet
        .rules
        .iter()
        .enumerate()
        .filter_map(|(rule, entry)| {
            entry
                .selectors
                .iter()
                .filter(|selector| matches(selector, &source.elements, &states, index))
                .map(specificity)
                .max()
                .map(|best| (best, rule))
        })
        .collect();
    matched.sort_unstable();
    matched.into_iter().map(|(_, rule)| rule).collect()
}

/// An Overlay's root is the screen.
fn root() -> ResolvedStyle {
    let mut style = ResolvedStyle::default();
    style.layout.size = Size {
        width: percent(1.0),
        height: percent(1.0),
    };
    style
}

/// `color` and `font-*` pass down; nothing else does.
fn inherited(parent: &ResolvedStyle) -> ResolvedStyle {
    let mut style = ResolvedStyle::default();
    style.paint.text = parent.paint.text;
    style.font_family.clone_from(&parent.font_family);
    style.font_size = parent.font_size;
    style
}

/// Descendant combinators only, so the nearest matching ancestor is always the right one to try.
fn matches(selector: &Selector, elements: &[Element], states: &[u8], index: usize) -> bool {
    let Some((subject, ancestors)) = selector.0.split_last() else {
        return false;
    };
    if !compound_matches(subject, elements, states, index) {
        return false;
    }
    let mut current = index;
    for compound in ancestors.iter().rev() {
        loop {
            let parent = elements[current].parent;
            if parent == NO_PARENT {
                return false;
            }
            current = parent as usize;
            if compound_matches(compound, elements, states, current) {
                break;
            }
        }
    }
    true
}

fn compound_matches(
    compound: &Compound,
    elements: &[Element],
    states: &[u8],
    index: usize,
) -> bool {
    let element = &elements[index];
    let root = if element.parent == NO_PARENT {
        STATE_ROOT
    } else {
        0
    };
    let states = states[index] | root;
    compound
        .element
        .as_deref()
        .is_none_or(|tag| tag == element.kind.tag())
        && compound
            .name
            .as_deref()
            .is_none_or(|name| element.name.as_deref() == Some(name))
        && compound
            .classes
            .iter()
            .all(|class| element.classes.contains(class))
        && compound.states & !states == 0
}

fn specificity(selector: &Selector) -> Specificity {
    selector
        .0
        .iter()
        .fold((0, 0, 0), |(names, classes, elements), c| {
            (
                names + u32::from(c.name.is_some()),
                classes + c.classes.len() as u32 + c.states.count_ones(),
                elements + u32::from(c.element.is_some()),
            )
        })
}
