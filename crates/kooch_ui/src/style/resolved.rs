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
    /// Its text as a share of the parent's height, over `font_size`.
    pub font_share: Option<f32>,
    /// The `kind: ui` shader; `None` when unset or a path no asset answers.
    pub material: Option<kooch_core::Guid>,
    /// `-kooch-param-*`, each name once.
    pub params: Vec<(Box<str>, [f32; 4])>,
    /// The resolved asset; `None` when unset, `none`, or a url no asset answers.
    pub image: Option<kooch_core::Guid>,
    pub mask: Option<kooch_core::Guid>,
    pub mask_luminance: bool,
    pub fill_from: u32,
    pub curve: Option<crate::UiPath>,
    pub flip: u32,
    pub transform: crate::UiTransform,
    /// `None`: the image's own, from its `.meta`.
    pub slice: Option<[f32; 4]>,
    pub tint: Option<[f32; 4]>,
}

// SAFETY: as for `UiTree`: taffy's lengths are `!Send` only for a `calc()` pointer, and taffy is
// built without `calc`, so no style holds one.
unsafe impl Send for ResolvedStyle {}
unsafe impl Sync for ResolvedStyle {}

impl ResolvedStyle {
    pub fn apply(&mut self, declaration: &Declaration) {
        let layout = &mut self.layout;
        match declaration {
            Declaration::Display(value) => layout.display = *value,
            Declaration::GridTemplate(Axis::Width, tracks) => {
                layout.grid_template_columns = tracks.clone()
            }
            Declaration::GridTemplate(Axis::Height, tracks) => {
                layout.grid_template_rows = tracks.clone()
            }
            Declaration::Overflow(value) => {
                layout.overflow = taffy::Point {
                    x: *value,
                    y: *value,
                }
            }
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
            Declaration::FontSize(size) => {
                self.font_size = Some(*size);
                self.font_share = None;
            }
            Declaration::FontShare(share) => self.font_share = Some(*share),
            Declaration::AspectRatio(ratio) => layout.aspect_ratio = *ratio,
            Declaration::Material(material) => self.material = material.guid,
            Declaration::Param(name, value) => {
                match self.params.iter_mut().find(|(own, _)| own == name) {
                    Some((_, own)) => *own = *value,
                    None => self.params.push((name.clone(), *value)),
                }
            }
            Declaration::BackgroundImage(image) => {
                self.image = image.as_ref().and_then(|image| image.guid);
            }
            Declaration::Mask(mask) => self.mask = mask.as_ref().and_then(|mask| mask.guid),
            Declaration::MaskLuminance(on) => self.mask_luminance = *on,
            Declaration::FillFrom(from) => self.fill_from = *from,
            Declaration::Curve(curve) => self.curve.clone_from(curve),
            Declaration::Flip(flip) => self.flip = *flip,
            Declaration::Rotate(degrees) => self.transform.rotate = *degrees,
            Declaration::RotateX(degrees) => self.transform.rotate_x = *degrees,
            Declaration::RotateY(degrees) => self.transform.rotate_y = *degrees,
            Declaration::Scale(scale) => self.transform.scale = *scale,
            Declaration::Translate(by) => self.transform.translate = *by,
            Declaration::TransformOrigin(origin) => self.transform.origin = *origin,
            Declaration::Perspective(distance) => self.transform.perspective = *distance,
            Declaration::Slice(slice) => self.slice = Some(*slice),
            Declaration::Tint(rgba) => self.tint = Some(*rgba),
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
