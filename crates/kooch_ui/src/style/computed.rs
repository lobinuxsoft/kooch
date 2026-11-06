//! A resolved style read back as CSS: what the UI Builder shows for a property nothing inline sets.

use std::fmt::Debug;

use taffy::style::{AlignContent, AlignItems, CompactLength};

use super::ResolvedStyle;
use crate::DEFAULT_FONT_SIZE;

/// `property`'s value on `style`, as a `.kcss` would write it. `None` for what has no text here:
/// the asset properties, a font nobody named, grid tracks a rule set.
pub fn computed(style: &ResolvedStyle, property: &str) -> Option<String> {
    let layout = &style.layout;
    let paint = &style.paint;
    Some(match property {
        "display" => keyword(layout.display),
        "overflow" => keyword(layout.overflow.x),
        "grid-template-columns" if layout.grid_template_columns.is_empty() => "none".to_owned(),
        "grid-template-rows" if layout.grid_template_rows.is_empty() => "none".to_owned(),
        "position" => keyword(layout.position),
        "top" => length(layout.inset.top.into_raw()),
        "right" => length(layout.inset.right.into_raw()),
        "bottom" => length(layout.inset.bottom.into_raw()),
        "left" => length(layout.inset.left.into_raw()),
        "flex-direction" => keyword(layout.flex_direction),
        // The one word CSS does not hyphenate.
        "flex-wrap" => keyword(layout.flex_wrap).replace("no-wrap", "nowrap"),
        "flex-grow" => number(layout.flex_grow),
        "flex-shrink" => number(layout.flex_shrink),
        "flex-basis" => length(layout.flex_basis.into_raw()),
        "gap" => length(layout.gap.width.into_raw()),
        "align-items" => items(layout.align_items),
        "justify-content" => content(layout.justify_content, "flex-start"),
        "align-self" => layout.align_self.map_or("stretch".to_owned(), items),
        "align-content" => content(layout.align_content, "stretch"),
        "width" => length(layout.size.width.into_raw()),
        "height" => length(layout.size.height.into_raw()),
        "min-width" => length(layout.min_size.width.into_raw()),
        "min-height" => length(layout.min_size.height.into_raw()),
        "max-width" => length(layout.max_size.width.into_raw()),
        "max-height" => length(layout.max_size.height.into_raw()),
        "margin-top" => length(layout.margin.top.into_raw()),
        "margin-right" => length(layout.margin.right.into_raw()),
        "margin-bottom" => length(layout.margin.bottom.into_raw()),
        "margin-left" => length(layout.margin.left.into_raw()),
        "padding-top" => length(layout.padding.top.into_raw()),
        "padding-right" => length(layout.padding.right.into_raw()),
        "padding-bottom" => length(layout.padding.bottom.into_raw()),
        "padding-left" => length(layout.padding.left.into_raw()),
        "border-top-width" => length(layout.border.top.into_raw()),
        "border-right-width" => length(layout.border.right.into_raw()),
        "border-bottom-width" => length(layout.border.bottom.into_raw()),
        "border-left-width" => length(layout.border.left.into_raw()),
        "border-radius" => format!("{}px", number(paint.radius)),
        "color" => color(paint.text),
        "background-color" => color(paint.background),
        "border-color" => color(paint.border),
        "-kooch-mask-mode" => match style.mask_luminance {
            true => "luminance".to_owned(),
            false => "alpha".to_owned(),
        },
        "-kooch-tint" => color(style.tint.unwrap_or([1.0; 4])),
        "font-size" => format!("{}px", number(style.font_size.unwrap_or(DEFAULT_FONT_SIZE))),
        "font-family" => format!("\"{}\"", style.font_family.as_ref()?),
        "-kooch-slice" => {
            let [top, right, bottom, left] = style.slice.map(number);
            format!("{top}px {right}px {bottom}px {left}px")
        }
        _ => return None,
    })
}

/// `FlexStart` as `flex-start`: taffy's variants are the CSS words in Pascal case.
fn keyword(value: impl Debug) -> String {
    let mut word = String::new();
    for letter in format!("{value:?}").chars() {
        if letter.is_uppercase() && !word.is_empty() {
            word.push('-');
        }
        word.push(letter.to_ascii_lowercase());
    }
    word
}

/// `normal` is taffy's word for what flexbox does untold, which no field lists.
fn items(value: AlignItems) -> String {
    match value == AlignItems::NORMAL {
        true => "stretch".to_owned(),
        false => keyword(value.keyword),
    }
}

fn content(value: AlignContent, normal: &str) -> String {
    match value == AlignContent::NORMAL {
        true => normal.to_owned(),
        false => keyword(value.keyword),
    }
}

fn length(value: CompactLength) -> String {
    if value.is_auto() {
        return "auto".to_owned();
    }
    match value.tag() {
        CompactLength::PERCENT_TAG => format!("{}%", number(value.value() * 100.0)),
        _ => format!("{}px", number(value.value())),
    }
}

/// Three decimals at most, without trailing zeros.
fn number(value: f32) -> String {
    let text = format!("{value:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

fn color(rgba: [f32; 4]) -> String {
    let [r, g, b, a] = rgba.map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8);
    format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
}
