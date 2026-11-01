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

use AttributeKind::{Action, Binding, Bool, Number, Template};

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
const SLIDER: [AttributeSpec; 11] = [
    spec("value", Number, "0", "Where it starts"),
    spec("min", Number, "0", "Lowest value"),
    spec("max", Number, "1", "Highest value"),
    spec("step", Number, "0", "Snaps to it; 0 slides freely"),
    BIND,
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

impl ElementKind {
    /// The attributes this element reads besides `name`, `class` and `style`, in the order the
    /// Inspector lists them.
    pub fn attributes(self) -> &'static [AttributeSpec] {
        match self {
            Self::Ui => std::slice::from_ref(&CANCEL),
            Self::Panel | Self::Label => &[],
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
