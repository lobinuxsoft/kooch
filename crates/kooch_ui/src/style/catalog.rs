//! The properties the UI Builder edits as fields, grouped as Unity's Inspector groups its inlined
//! styles. Only longhands: a side is its own field, and a shorthand splits on the first edit.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyKind {
    /// One of the words, as written.
    Keyword(&'static [&'static str]),
    /// `px` or `%`, and `auto` when the property takes it.
    Length {
        auto: bool,
    },
    Number,
    /// Canvas units only.
    Px,
    Color,
    Text,
    /// A family the engine embeds or the project holds as a font file.
    Font,
    /// `url()` to an image asset, relative to the file writing it.
    Image,
    /// A quoted path to a `kind: ui` shader.
    Shader,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PropertySpec {
    pub name: &'static str,
    pub kind: PropertyKind,
    pub hint: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StyleCategory {
    pub title: &'static str,
    pub properties: &'static [PropertySpec],
}

const fn prop(name: &'static str, kind: PropertyKind, hint: &'static str) -> PropertySpec {
    PropertySpec { name, kind, hint }
}

use PropertyKind::{Color, Font, Image, Keyword, Length, Number, Px, Shader, Text};

const AUTO: PropertyKind = Length { auto: true };
const FIXED: PropertyKind = Length { auto: false };
const ALIGN_ITEMS: PropertyKind =
    Keyword(&["flex-start", "center", "flex-end", "stretch", "baseline"]);
const ALIGN_CONTENT: PropertyKind = Keyword(&[
    "flex-start",
    "center",
    "flex-end",
    "stretch",
    "space-between",
    "space-around",
    "space-evenly",
]);

pub const CATEGORIES: [StyleCategory; 10] = [
    StyleCategory {
        title: "Display",
        properties: &[
            prop(
                "display",
                Keyword(&["flex", "grid", "block", "none"]),
                "none takes it out of the layout",
            ),
            prop(
                "overflow",
                Keyword(&["visible", "hidden", "scroll"]),
                "hidden cuts its children at its edge; scroll also lets the wheel move them",
            ),
        ],
    },
    StyleCategory {
        title: "Position",
        properties: &[
            prop(
                "position",
                Keyword(&["relative", "absolute"]),
                "absolute leaves the flow; the insets place it in its parent",
            ),
            prop(
                "top",
                AUTO,
                "Distance from its parent's top edge; with position: absolute, where it sits",
            ),
            prop(
                "right",
                AUTO,
                "Distance from its parent's right edge; with position: absolute, where it sits",
            ),
            prop(
                "bottom",
                AUTO,
                "Distance from its parent's bottom edge; with position: absolute, where it sits",
            ),
            prop(
                "left",
                AUTO,
                "Distance from its parent's left edge; with position: absolute, where it sits",
            ),
        ],
    },
    StyleCategory {
        title: "Flex",
        properties: &[
            prop(
                "flex-direction",
                Keyword(&["row", "column", "row-reverse", "column-reverse"]),
                "The axis its children line up on",
            ),
            prop(
                "flex-wrap",
                Keyword(&["nowrap", "wrap", "wrap-reverse"]),
                "wrap breaks its children into more lines when they do not fit",
            ),
            prop("flex-grow", Number, "Share of the free space it takes"),
            prop("flex-shrink", Number, "Share of the overflow it gives up"),
            prop("flex-basis", AUTO, "Its size before growing or shrinking"),
            prop("gap", FIXED, "Space between its children"),
        ],
    },
    StyleCategory {
        title: "Align",
        properties: &[
            prop("align-items", ALIGN_ITEMS, "Its children across the axis"),
            prop(
                "justify-content",
                ALIGN_CONTENT,
                "Its children along the axis",
            ),
            prop(
                "align-self",
                ALIGN_ITEMS,
                "Itself, over its parent's align-items",
            ),
            prop("align-content", ALIGN_CONTENT, "Its wrapped lines"),
        ],
    },
    StyleCategory {
        title: "Size",
        properties: &[
            prop(
                "width",
                AUTO,
                "Its width; auto fits its content, % is of its parent's",
            ),
            prop(
                "height",
                AUTO,
                "Its height; auto fits its content, % is of its parent's",
            ),
            prop("min-width", AUTO, "It never gets narrower than this"),
            prop("min-height", AUTO, "It never gets shorter than this"),
            prop("max-width", AUTO, "It never gets wider than this"),
            prop("max-height", AUTO, "It never gets taller than this"),
        ],
    },
    StyleCategory {
        title: "Margin",
        properties: &[
            prop(
                "margin-top",
                AUTO,
                "Space outside it, above; auto pushes it down",
            ),
            prop(
                "margin-right",
                AUTO,
                "Space outside it, to the right; auto pushes it left",
            ),
            prop(
                "margin-bottom",
                AUTO,
                "Space outside it, below; auto pushes it up",
            ),
            prop(
                "margin-left",
                AUTO,
                "Space outside it, to the left; auto pushes it right",
            ),
        ],
    },
    StyleCategory {
        title: "Padding",
        properties: &[
            prop(
                "padding-top",
                FIXED,
                "Space inside it, between its top edge and its children",
            ),
            prop(
                "padding-right",
                FIXED,
                "Space inside it, between its right edge and its children",
            ),
            prop(
                "padding-bottom",
                FIXED,
                "Space inside it, between its bottom edge and its children",
            ),
            prop(
                "padding-left",
                FIXED,
                "Space inside it, between its left edge and its children",
            ),
        ],
    },
    StyleCategory {
        title: "Text",
        properties: &[
            prop("color", Color, "Passes down to its children"),
            prop(
                "font-family",
                Font,
                "A family of the engine or the project; passes down",
            ),
            prop("font-size", Px, "Passes down"),
        ],
    },
    StyleCategory {
        title: "Background",
        properties: &[
            prop(
                "background-color",
                Color,
                "Fills its box, under the background image",
            ),
            prop(
                "background-image",
                Image,
                "url(\"image.png\"), relative to the file writing it; none clears it",
            ),
            prop("-kooch-tint", Color, "Multiplies the image"),
            prop(
                "-kooch-slice",
                Text,
                "Texels kept unstretched: top right bottom left",
            ),
            prop("-kooch-material", Shader, "\"path.shader\" with kind: ui"),
        ],
    },
    StyleCategory {
        title: "Border",
        properties: &[
            prop("border-color", Color, "The border's colour"),
            prop("border-top-width", FIXED, "The border's thickness on top"),
            prop(
                "border-right-width",
                FIXED,
                "The border's thickness on the right",
            ),
            prop(
                "border-bottom-width",
                FIXED,
                "The border's thickness at the bottom",
            ),
            prop(
                "border-left-width",
                FIXED,
                "The border's thickness on the left",
            ),
            prop(
                "border-radius",
                Px,
                "Rounds its corners, background and border",
            ),
        ],
    },
];
