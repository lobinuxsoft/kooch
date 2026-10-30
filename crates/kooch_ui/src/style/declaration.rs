//! One property's value, typed as what it sets: a taffy field or the node's paint.

use std::str::FromStr;

use cssparser::{Parser, ParserInput, Token};
use cssparser_color::Color;
use taffy::{
    AlignContent, AlignItems, Dimension, Display, FlexDirection, FlexWrap, LengthPercentage,
    LengthPercentageAuto, Position,
};

use super::tokens::components;

/// A box side, CSS order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Top,
    Right,
    Bottom,
    Left,
}

/// `Width` is also the column gap, `Height` the row gap — taffy's `Size` layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Width,
    Height,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Declaration {
    Display(Display),
    Position(Position),
    Inset(Edge, LengthPercentageAuto),
    Size(Axis, Dimension),
    MinSize(Axis, LengthPercentageAuto),
    MaxSize(Axis, LengthPercentageAuto),
    FlexDirection(FlexDirection),
    FlexWrap(FlexWrap),
    FlexGrow(f32),
    FlexShrink(f32),
    FlexBasis(Dimension),
    AlignItems(AlignItems),
    AlignSelf(AlignItems),
    AlignContent(AlignContent),
    JustifyContent(AlignContent),
    Gap(Axis, LengthPercentage),
    Margin(Edge, LengthPercentageAuto),
    Padding(Edge, LengthPercentage),
    BorderWidth(Edge, LengthPercentage),
    /// sRGB, straight alpha, like `Paint`.
    Background([f32; 4]),
    Color([f32; 4]),
    BorderColor([f32; 4]),
    /// Canvas units.
    BorderRadius(f32),
    FontFamily(Box<str>),
    FontSize(f32),
    /// A `.shader` path with `kind: ui` (#1690).
    Material(Box<str>),
}

const EDGES: [Edge; 4] = [Edge::Top, Edge::Right, Edge::Bottom, Edge::Left];

/// `name: text`, `var()` already substituted. A shorthand is several declarations.
pub(super) fn parse(name: &str, text: &str) -> Result<Vec<Declaration>, String> {
    use Declaration as D;
    let one = |declaration| Ok(vec![declaration]);
    let parsed: Result<Vec<Declaration>, String> = match name {
        "display" => one(D::Display(layout(text)?)),
        "position" => one(D::Position(layout(text)?)),
        "top" | "right" | "bottom" | "left" => one(D::Inset(edge(name)?, layout(text)?)),
        "width" => one(D::Size(Axis::Width, layout(text)?)),
        "height" => one(D::Size(Axis::Height, layout(text)?)),
        "min-width" => one(D::MinSize(Axis::Width, layout(text)?)),
        "min-height" => one(D::MinSize(Axis::Height, layout(text)?)),
        "max-width" => one(D::MaxSize(Axis::Width, layout(text)?)),
        "max-height" => one(D::MaxSize(Axis::Height, layout(text)?)),
        "flex-direction" => one(D::FlexDirection(layout(text)?)),
        "flex-wrap" => one(D::FlexWrap(layout(text)?)),
        "flex-grow" => one(D::FlexGrow(number(text)?)),
        "flex-shrink" => one(D::FlexShrink(number(text)?)),
        "flex-basis" => one(D::FlexBasis(layout(text)?)),
        "align-items" => one(D::AlignItems(layout(text)?)),
        "align-self" => one(D::AlignSelf(layout(text)?)),
        "align-content" => one(D::AlignContent(layout(text)?)),
        "justify-content" => one(D::JustifyContent(layout(text)?)),
        "gap" => {
            let parts = components(text);
            let (row, column) = match parts.as_slice() {
                [both] => (layout(both)?, layout(both)?),
                [row, column] => (layout(row)?, layout(column)?),
                _ => return Err(format!("`{text}` is one or two lengths")),
            };
            Ok(vec![D::Gap(Axis::Height, row), D::Gap(Axis::Width, column)])
        }
        "margin" => Ok(sides(text)?
            .into_iter()
            .map(|(e, v)| D::Margin(e, v))
            .collect()),
        "padding" => Ok(sides(text)?
            .into_iter()
            .map(|(e, v)| D::Padding(e, v))
            .collect()),
        "border-width" => Ok(sides(text)?
            .into_iter()
            .map(|(e, v)| D::BorderWidth(e, v))
            .collect()),
        _ if name.starts_with("margin-") => one(D::Margin(edge(&name[7..])?, layout(text)?)),
        _ if name.starts_with("padding-") => one(D::Padding(edge(&name[8..])?, layout(text)?)),
        "background-color" => one(D::Background(color(text)?)),
        "color" => one(D::Color(color(text)?)),
        "border-color" => one(D::BorderColor(color(text)?)),
        "border" => border(text),
        "border-radius" => one(D::BorderRadius(px(text)?)),
        "font-family" => one(D::FontFamily(family(text)?)),
        "font-size" => one(D::FontSize(px(text)?)),
        "material" => one(D::Material(quoted(text)?)),
        _ => return Err(format!("`{name}` is not a .kooch_style property")),
    };
    parsed.map_err(|message| format!("{name}: {message}"))
}

fn edge(side: &str) -> Result<Edge, String> {
    match side {
        "top" => Ok(Edge::Top),
        "right" => Ok(Edge::Right),
        "bottom" => Ok(Edge::Bottom),
        "left" => Ok(Edge::Left),
        _ => Err(format!("`{side}` is not a side")),
    }
}

/// Through taffy's parser. CSS lets a bare `0` stand for a length; taffy wants the unit.
fn layout<T: FromStr>(text: &str) -> Result<T, String> {
    let text = text.trim();
    let text = if text == "0" { "0px" } else { text };
    text.parse()
        .map_err(|_| format!("`{text}` is not a valid value"))
}

/// One to four values, top-right-bottom-left as in CSS.
fn sides<T: FromStr + Clone>(text: &str) -> Result<Vec<(Edge, T)>, String> {
    let values = components(text)
        .iter()
        .map(|part| layout::<T>(part))
        .collect::<Result<Vec<_>, _>>()?;
    let [top, right, bottom, left] = match values.as_slice() {
        [all] => [all, all, all, all],
        [vertical, horizontal] => [vertical, horizontal, vertical, horizontal],
        [top, horizontal, bottom] => [top, horizontal, bottom, horizontal],
        [top, right, bottom, left] => [top, right, bottom, left],
        _ => return Err(format!("`{text}` is one to four lengths")),
    };
    Ok(EDGES
        .into_iter()
        .zip([top, right, bottom, left].map(Clone::clone))
        .collect())
}

/// `<width> <colour>`, in either order.
fn border(text: &str) -> Result<Vec<Declaration>, String> {
    let mut out = Vec::new();
    for part in components(text) {
        if let Ok(width) = layout::<LengthPercentage>(&part) {
            out.extend(EDGES.map(|edge| Declaration::BorderWidth(edge, width)));
        } else {
            out.push(Declaration::BorderColor(color(&part)?));
        }
    }
    Ok(out)
}

fn color(text: &str) -> Result<[f32; 4], String> {
    let mut input = ParserInput::new(text);
    let mut parser = Parser::new(&mut input);
    match parser.parse_entirely(Color::parse) {
        Ok(Color::Rgba(rgba)) => Ok([
            f32::from(rgba.red) / 255.0,
            f32::from(rgba.green) / 255.0,
            f32::from(rgba.blue) / 255.0,
            rgba.alpha,
        ]),
        Ok(_) => Err(format!(
            "`{text}`: only #hex, rgb(), rgba() and named colours"
        )),
        Err(_) => Err(format!("`{text}` is not a colour")),
    }
}

fn number(text: &str) -> Result<f32, String> {
    text.trim()
        .parse()
        .map_err(|_| format!("`{text}` is not a number"))
}

/// A length in `px`: canvas units, not physical pixels.
fn px(text: &str) -> Result<f32, String> {
    let mut input = ParserInput::new(text);
    let mut parser = Parser::new(&mut input);
    let parsed: Result<f32, cssparser::ParseError<'_, ()>> = parser.parse_entirely(|parser| {
        let token = parser.next()?.clone();
        match token {
            Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("px") => Ok(value),
            Token::Number { value, .. } if value == 0.0 => Ok(0.0),
            token => Err(parser.new_unexpected_token_error(token)),
        }
    });
    parsed.map_err(|_| format!("`{text}` is not a length in px"))
}

fn quoted(text: &str) -> Result<Box<str>, String> {
    let mut input = ParserInput::new(text);
    let mut parser = Parser::new(&mut input);
    let parsed: Result<Box<str>, cssparser::ParseError<'_, ()>> =
        parser.parse_entirely(|parser| Ok(parser.expect_string()?.as_ref().into()));
    parsed.map_err(|_| format!("`{text}` is not a quoted string"))
}

/// A quoted name, or bare words joined as CSS does.
fn family(text: &str) -> Result<Box<str>, String> {
    if let Ok(name) = quoted(text) {
        return Ok(name);
    }
    let words = components(text);
    if words.is_empty()
        || words
            .iter()
            .any(|word| !word.chars().all(|c| c.is_alphanumeric() || c == '-'))
    {
        return Err(format!("`{text}` is not a font name"));
    }
    Ok(words.join(" ").into())
}
