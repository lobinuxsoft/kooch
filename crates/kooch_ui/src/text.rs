//! Text: parley shapes and breaks lines, the [`UiTree`](crate::UiTree) measures with it, and
//! [`GlyphAtlas`](crate::GlyphAtlas) holds the rasterized glyphs. The stack of `bevy_text` 0.19.

use std::borrow::Cow;
use std::sync::Arc;

use parley::fontique::{Blob, GenericFamily};
use parley::{
    Alignment, AlignmentOptions, FontContext, FontFamily, FontFamilyName, Layout, LayoutContext,
    StyleProperty,
};

/// The engine's UI font, Inter (SIL OFL 1.1, `assets/fonts/Inter-OFL.txt`): variable, every weight
/// in one file.
static INTER: &[u8] = include_bytes!("../assets/fonts/Inter.ttf");
/// Fira Code (SIL OFL 1.1, `assets/fonts/FiraCode-OFL.txt`): monospace with programming ligatures,
/// asked for by family.
static FIRA_CODE: &[u8] = include_bytes!("../assets/fonts/FiraCode-Regular.ttf");

/// Inter first; the system's sans-serif for any glyph it lacks.
const FAMILY: &[FontFamilyName<'static>] = &[
    FontFamilyName::Named(Cow::Borrowed("Inter")),
    FontFamilyName::Generic(GenericFamily::SansSerif),
];

/// Fonts and the shaping scratch space, shared by every document.
pub struct UiFonts {
    pub(crate) font: FontContext,
    pub(crate) layout: LayoutContext<()>,
    pub(crate) scale: swash::scale::ScaleContext,
}

impl Default for UiFonts {
    fn default() -> Self {
        let mut font = FontContext::new();
        for bytes in [INTER, FIRA_CODE] {
            let data: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(bytes);
            font.collection.register_fonts(Blob::new(data), None);
        }
        Self {
            font,
            layout: LayoutContext::new(),
            scale: swash::scale::ScaleContext::new(),
        }
    }
}

/// One node's text, shaped in logical units.
pub(crate) struct Text {
    pub content: String,
    pub size: f32,
    /// A family tried before Inter, `"Fira Code"`; `None` is Inter.
    pub family: Option<Box<str>>,
    /// `None` until shaped, and again whenever the content or size changes.
    pub layout: Option<Layout<()>>,
    /// Top-left of the content box, in the canvas: where the first line starts.
    pub origin: glam::Vec2,
}

impl std::fmt::Debug for Text {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Text")
            .field("content", &self.content)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl Text {
    pub fn shaped(&mut self, fonts: &mut UiFonts) -> &mut Layout<()> {
        self.layout.get_or_insert_with(|| {
            let mut builder =
                fonts
                    .layout
                    .ranged_builder(&mut fonts.font, &self.content, 1.0, true);
            let family = match &self.family {
                Some(first) => {
                    let mut list = vec![FontFamilyName::Named(Cow::Borrowed(first))];
                    list.extend_from_slice(FAMILY);
                    FontFamily::List(Cow::Owned(list))
                }
                None => FontFamily::List(Cow::Borrowed(FAMILY)),
            };
            builder.push_default(StyleProperty::FontFamily(family));
            builder.push_default(StyleProperty::FontSize(self.size));
            builder.build(&self.content)
        })
    }
}

/// Breaks `layout` to `width` and returns its size: what taffy asks of a text leaf.
pub(crate) fn measure(layout: &mut Layout<()>, width: Option<f32>) -> glam::Vec2 {
    layout.break_all_lines(width);
    layout.align(Alignment::Start, AlignmentOptions::default());
    glam::Vec2::new(layout.width(), layout.height())
}
