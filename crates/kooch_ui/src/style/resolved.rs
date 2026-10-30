//! Declarations folded into what a node is drawn and laid out with.

use taffy::{Rect, Size};

use super::declaration::{Axis, Declaration, Edge};
use crate::Paint;

/// One node's style once its rules are applied, in order. Font fields stay `None` when unset, so
/// inheritance (#280 phase 4) can tell unset from set.
#[derive(Debug, Clone, Default)]
pub struct ResolvedStyle {
    pub layout: taffy::Style,
    pub paint: Paint,
    pub font_family: Option<Box<str>>,
    pub font_size: Option<f32>,
    pub material: Option<Box<str>>,
}

impl ResolvedStyle {
    pub fn apply(&mut self, declaration: &Declaration) {
        let layout = &mut self.layout;
        match declaration {
            Declaration::Display(value) => layout.display = *value,
            Declaration::Position(value) => layout.position = *value,
            Declaration::Inset(edge, value) => *side(&mut layout.inset, *edge) = *value,
            Declaration::Size(axis, value) => *extent(&mut layout.size, *axis) = *value,
            Declaration::MinSize(axis, value) => *extent(&mut layout.min_size, *axis) = *value,
            Declaration::MaxSize(axis, value) => *extent(&mut layout.max_size, *axis) = *value,
            Declaration::FlexDirection(value) => layout.flex_direction = *value,
            Declaration::FlexWrap(value) => layout.flex_wrap = *value,
            Declaration::FlexGrow(value) => layout.flex_grow = *value,
            Declaration::FlexShrink(value) => layout.flex_shrink = *value,
            Declaration::FlexBasis(value) => layout.flex_basis = *value,
            Declaration::AlignItems(value) => layout.align_items = *value,
            Declaration::AlignSelf(value) => layout.align_self = Some(*value),
            Declaration::AlignContent(value) => layout.align_content = *value,
            Declaration::JustifyContent(value) => layout.justify_content = *value,
            Declaration::Gap(axis, value) => *extent(&mut layout.gap, *axis) = *value,
            Declaration::Margin(edge, value) => *side(&mut layout.margin, *edge) = *value,
            Declaration::Padding(edge, value) => *side(&mut layout.padding, *edge) = *value,
            Declaration::BorderWidth(edge, value) => *side(&mut layout.border, *edge) = *value,
            Declaration::Background(rgba) => self.paint.background = *rgba,
            Declaration::Color(rgba) => self.paint.text = *rgba,
            Declaration::BorderColor(rgba) => self.paint.border = *rgba,
            Declaration::BorderRadius(radius) => self.paint.radius = *radius,
            Declaration::FontFamily(family) => self.font_family = Some(family.clone()),
            Declaration::FontSize(size) => self.font_size = Some(*size),
            Declaration::Material(path) => self.material = Some(path.clone()),
        }
    }
}

fn side<T>(rect: &mut Rect<T>, edge: Edge) -> &mut T {
    match edge {
        Edge::Top => &mut rect.top,
        Edge::Right => &mut rect.right,
        Edge::Bottom => &mut rect.bottom,
        Edge::Left => &mut rect.left,
    }
}

fn extent<T>(size: &mut Size<T>, axis: Axis) -> &mut T {
    match axis {
        Axis::Width => &mut size.width,
        Axis::Height => &mut size.height,
    }
}
