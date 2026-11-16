//! The attributes each element reads, with their kind and default: what the UI Builder's Inspector
//! shows as fields, as Unity's reads an element's `UxmlTraits`.

use crate::source::ElementKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributeKind {
    /// `"true"` / `"false"`; absent reads as the default.
    Bool,
    Number,
    Text,
    /// An action name the game registers (`on:*`).
    Action,
    /// `Type.field` of a component on the document's entity.
    Binding,
    /// A `.kxml` path, relative to the file.
    Template,
    /// A `.kcss` path, relative to the file, or `default`.
    Sheet,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AttributeSpec {
    pub key: &'static str,
    pub kind: AttributeKind,
    /// What the element reads when the attribute is absent, as written.
    pub default: &'static str,
    pub hint: &'static str,
}

const fn spec(
    key: &'static str,
    kind: AttributeKind,
    default: &'static str,
    hint: &'static str,
) -> AttributeSpec {
    AttributeSpec {
        key,
        kind,
        default,
        hint,
    }
}

use AttributeKind::{Action, Binding, Bool, Number, Sheet, Template, Text};

const FOCUS: [AttributeSpec; 5] = [
    spec(
        "disabled",
        Bool,
        "false",
        "Neither clicks nor takes focus; selects `:disabled`",
    ),
    spec(
        "autofocus",
        Bool,
        "false",
        "Takes focus each time the document appears",
    ),
    spec("tabindex", Number, "0", "-1 keeps it out of focus"),
    spec("on:focus", Action, "", "Sent when it takes focus"),
    spec("on:blur", Action, "", "Sent when it loses focus"),
];
const CANCEL: AttributeSpec = spec(
    "on:cancel",
    Action,
    "",
    "Sent on Cancel while focus is inside",
);
const CLICK: AttributeSpec = spec("on:click", Action, "", "Sent on a click or Submit");
const CHANGE: AttributeSpec = spec("on:change", Action, "", "Sent with the new value");
const BIND: AttributeSpec = spec(
    "bind:value",
    Binding,
    "",
    "Kept in step with a component's field",
);

const BUTTON: [AttributeSpec; 7] = [
    CLICK, CANCEL, FOCUS[0], FOCUS[1], FOCUS[2], FOCUS[3], FOCUS[4],
];
const TOGGLE: [AttributeSpec; 8] = [
    spec("checked", Bool, "false", "Starts on"),
    BIND,
    CHANGE,
    FOCUS[0],
    FOCUS[1],
    FOCUS[2],
    FOCUS[3],
    FOCUS[4],
];
/// What a node showing a `.webm` reads (#1793).
const VIDEO: [AttributeSpec; 3] = [
    spec(
        "autoplay",
        Bool,
        "true",
        "Plays its video as soon as it shows",
    ),
    spec("loop", Bool, "true", "Starts its video over at the end"),
    spec(
        "on:end",
        Action,
        "",
        "Sent when a video that does not loop ends",
    ),
];
const TIME: [AttributeSpec; 1] = [spec(
    "bind:video-time",
    Text,
    "",
    "Shows `m:ss / m:ss` of the video node with this name",
)];
const SLIDER: [AttributeSpec; 12] = [
    spec("value", Number, "0", "Where it starts"),
    spec("min", Number, "0", "Lowest value"),
    spec("max", Number, "1", "Highest value"),
    spec("step", Number, "0", "Snaps to it; 0 slides freely"),
    BIND,
    spec(
        "bind:video",
        Text,
        "",
        "Follows the video node with this name, and seeks it when dragged",
    ),
    CHANGE,
    FOCUS[0],
    FOCUS[1],
    FOCUS[2],
    FOCUS[3],
    FOCUS[4],
];
const USE: [AttributeSpec; 2] = [
    spec("src", Template, "", "The template instanced here"),
    CLICK,
];

const UI: [AttributeSpec; 2] = [
    CANCEL,
    spec(
        "sheet",
        Sheet,
        "",
        "Used as a template it wears this sheet, not the document's; unset takes the document's",
    ),
];

impl ElementKind {
    /// Takes child elements: what the UI Builder drops into.
    pub fn holds_children(self) -> bool {
        matches!(self, Self::Ui | Self::Panel | Self::Button)
    }

    /// The attributes this element reads besides `name`, `class` and `style`, in the order the
    /// Inspector lists them.
    pub fn attributes(self) -> &'static [AttributeSpec] {
        match self {
            Self::Ui => &UI,
            Self::Panel => &VIDEO,
            Self::Label => &TIME,
            Self::Button => &BUTTON,
            Self::Toggle => &TOGGLE,
            Self::Slider => &SLIDER,
            Self::Use => &USE,
        }
    }

    pub fn from_tag(tag: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.tag() == tag)
    }
}
