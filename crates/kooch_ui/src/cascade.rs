//! Which rules reach which element, and in what order. States (`:hover`…) wait for UI input
//! (#96 phase 6): a rule that needs one does not apply yet.

use taffy::Size;
use taffy::prelude::percent;

use crate::source::{Element, NO_PARENT, UiSource};
use crate::style::{Compound, ResolvedStyle, STATE_ROOT, Selector, UiStyleSheet};

/// CSS's: names, then classes and states, then elements.
type Specificity = (u32, u32, u32);

/// One style per element, in `source.elements` order.
pub fn resolve(source: &UiSource, sheet: Option<&UiStyleSheet>) -> Vec<ResolvedStyle> {
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
                    .filter(|selector| matches(selector, &source.elements, index))
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
        resolved.push(style);
    }
    resolved
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
fn matches(selector: &Selector, elements: &[Element], index: usize) -> bool {
    let Some((subject, ancestors)) = selector.0.split_last() else {
        return false;
    };
    if !compound_matches(subject, elements, index) {
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
            if compound_matches(compound, elements, current) {
                break;
            }
        }
    }
    true
}

fn compound_matches(compound: &Compound, elements: &[Element], index: usize) -> bool {
    let element = &elements[index];
    let states = if element.parent == NO_PARENT {
        STATE_ROOT
    } else {
        0
    };
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
