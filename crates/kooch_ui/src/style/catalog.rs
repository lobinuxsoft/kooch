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

use PropertyKind::{Color, Keyword, Length, Number, Px, Text};

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
        properties: &[prop(
            "display",
            Keyword(&["flex", "grid", "block", "none"]),
            "none takes it out of the layout",
        )],
    },
    StyleCategory {
        title: "Position",
        properties: &[
            prop(
                "position",
                Keyword(&["relative", "absolute"]),
                "absolute leaves the flow; the insets place it in its parent",
            ),
            prop("top", AUTO, ""),
            prop("right", AUTO, ""),
            prop("bottom", AUTO, ""),
            prop("left", AUTO, ""),
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
                "",
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
            prop("width", AUTO, ""),
            prop("height", AUTO, ""),
            prop("min-width", AUTO, ""),
            prop("min-height", AUTO, ""),
            prop("max-width", AUTO, ""),
            prop("max-height", AUTO, ""),
        ],
    },
    StyleCategory {
        title: "Margin",
        properties: &[
            prop("margin-top", AUTO, ""),
            prop("margin-right", AUTO, ""),
            prop("margin-bottom", AUTO, ""),
            prop("margin-left", AUTO, ""),
        ],
    },
    StyleCategory {
        title: "Padding",
        properties: &[
            prop("padding-top", FIXED, ""),
            prop("padding-right", FIXED, ""),
            prop("padding-bottom", FIXED, ""),
            prop("padding-left", FIXED, ""),
        ],
    },
    StyleCategory {
        title: "Text",
        properties: &[
            prop("color", Color, "Passes down to its children"),
            prop("font-family", Text, "A font of the project; passes down"),
            prop("font-size", Px, "Passes down"),
        ],
    },
    StyleCategory {
        title: "Background",
        properties: &[
            prop("background-color", Color, ""),
            prop(
                "background-image",
                Text,
                "url(\"image.png\"), relative to the sheet; none clears it",
            ),
            prop("-kooch-tint", Color, "Multiplies the image"),
            prop(
                "-kooch-slice",
                Text,
                "Texels kept unstretched: top right bottom left",
            ),
            prop("-kooch-material", Text, "\"path.shader\" with kind: ui"),
        ],
    },
    StyleCategory {
        title: "Border",
        properties: &[
            prop("border-color", Color, ""),
            prop("border-top-width", FIXED, ""),
            prop("border-right-width", FIXED, ""),
            prop("border-bottom-width", FIXED, ""),
            prop("border-left-width", FIXED, ""),
            prop("border-radius", Px, ""),
        ],
    },
];
