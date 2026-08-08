//! [`Binding`] — one control feeding an action, and how several of them
//! combine.
//!
//! # The flat list
//!
//! An action's bindings are a `Vec`, and a composite is a **head entry
//! followed by its parts**:
//!
//! ```text
//! [ Whole(Space)                  ← jump, plain binding
//!   CompositeHead(Vector2 { .. }) ← move, the composite itself
//!   Part(Up,    Key(KeyW))
//!   Part(Down,  Key(KeyS))
//!   Part(Left,  Key(KeyA))
//!   Part(Right, Key(KeyD))
//!   CompositeHead(Vector2 { .. }) ← move again, this time the stick
//!   Part(Up,    Axis(LeftStickY))
//!   … ]
//! ```
//!
//! Unity's `.inputactions` does exactly this, with two booleans
//! (`isComposite`, `isPartOfComposite`) where this has one enum. It reads
//! like an odd choice until you notice it is also the right one for us: a
//! contiguous array, no tree, no boxed nodes, and the editor's list *is*
//! the data.
//!
//! Two booleans allow a state that means nothing (both true); an enum
//! does not, which is the same argument that kept `PhysicsBody` a single
//! component with a `kind` instead of three components.

use serde::{Deserialize, Serialize};

use super::path::ControlPath;
use super::processor::Processor;

/// Which part of a composite a binding plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PartName {
    /// The positive side of a 1D axis, or the up/right of a 2D vector.
    Positive,
    Negative,
    Up,
    Down,
    Left,
    Right,
    /// +Z and −Z of a 3D vector.
    Forward,
    Backward,
    /// The button that gates a modifier composite — ctrl in `Ctrl+S`.
    Modifier,
    /// The second gate of [`Composite::TwoModifiers`].
    Modifier2,
    /// What a modifier composite reads once its gates are held. Unity
    /// calls this part `binding`; named for what it carries here, since
    /// `Binding` is already the type every part is one of.
    Value,
}

impl PartName {
    /// The parts a composite expects, in the order an editor lists them.
    ///
    /// Drives both the "add composite" flow, which creates one unbound
    /// part per name, and the panel, which shows what a composite is
    /// still missing.
    pub const fn of(composite: Composite) -> &'static [Self] {
        match composite {
            Composite::Axis1D { .. } => &[Self::Positive, Self::Negative],
            Composite::Vector2 { .. } => &[Self::Up, Self::Down, Self::Left, Self::Right],
            Composite::Vector3 { .. } => &[
                Self::Up,
                Self::Down,
                Self::Left,
                Self::Right,
                Self::Forward,
                Self::Backward,
            ],
            Composite::OneModifier => &[Self::Modifier, Self::Value],
            Composite::TwoModifiers => &[Self::Modifier, Self::Modifier2, Self::Value],
        }
    }
}

/// How a composite turns its parts into one value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Composite {
    /// Two buttons into one axis: [`PartName::Positive`] and
    /// [`PartName::Negative`].
    Axis1D {
        /// What happens when both sides are held. Unity calls this
        /// `whichSideWins`, and the default — neither — is the one that
        /// makes a keyboard behave: press left and right together and you
        /// stand still, rather than drifting whichever way the code
        /// happened to check first.
        both_held: BothHeld,
    },
    /// Four buttons, or two axes, into a vector.
    Vector2 { mode: VectorMode },
    /// Six buttons, or three axes, into a 3D vector. Unity's
    /// `Vector3Composite` — the shape a flying or free-floating
    /// controller wants, where up and down are inputs rather than gravity.
    Vector3 { mode: VectorMode },
    /// A control gated by a held button: [`PartName::Value`] passes
    /// through only while [`PartName::Modifier`] is down.
    ///
    /// This is `Ctrl+S`. Without it every shortcut has to be spelled out
    /// in gameplay code, which is exactly the branching an action map
    /// exists to delete.
    OneModifier,
    /// The same, gated by two — `Ctrl+Shift+S`.
    TwoModifiers,
}

impl Composite {
    /// Every composite an editor can offer, in menu order.
    pub const ALL: &'static [Self] = &[
        Self::Vector2 {
            mode: VectorMode::DigitalNormalized,
        },
        Self::Axis1D {
            both_held: BothHeld::Neither,
        },
        Self::Vector3 {
            mode: VectorMode::DigitalNormalized,
        },
        Self::OneModifier,
        Self::TwoModifiers,
    ];

    /// What this produces, so an editor can offer only the composites
    /// that fit the action being edited.
    ///
    /// Unity filters its "Add Composite" menu the same way, and the
    /// reason is that the alternative — a Vector2 composite under a
    /// Button action — is a binding that silently reads as nothing.
    pub const fn control_type(self) -> super::action::ControlType {
        use super::action::ControlType;
        match self {
            Self::Vector2 { .. } => ControlType::Vector2,
            Self::Vector3 { .. } => ControlType::Vector3,
            Self::Axis1D { .. } => ControlType::Axis,
            // Whatever the gated part reads. Button is the honest
            // default: `Ctrl+S` is a button, and an axis behind a
            // modifier still passes its own magnitude through.
            Self::OneModifier | Self::TwoModifiers => ControlType::Button,
        }
    }

    /// Name for a menu entry.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Axis1D { .. } => "1D Axis",
            Self::Vector2 { .. } => "2D Vector",
            Self::Vector3 { .. } => "3D Vector",
            Self::OneModifier => "One Modifier",
            Self::TwoModifiers => "Two Modifiers",
        }
    }
}

/// Resolution when both sides of an axis are held at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BothHeld {
    /// They cancel. The default, and what a keyboard should do.
    #[default]
    Neither,
    Positive,
    Negative,
}

/// How a vector composite reads its parts. Shared by 2D and 3D, as
/// Unity's two separate `Mode` enums are the same three cases.
///
/// The variant names are serialised, not this type name, so renaming it
/// from `Vector2Mode` does not invalidate any `.inputmap` on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum VectorMode {
    /// Parts are buttons, and the result is capped at length 1 — so a
    /// diagonal does not travel 1.41× faster than a straight line. The
    /// default, and what WASD needs.
    #[default]
    DigitalNormalized,
    /// Parts are buttons, uncapped. A diagonal is longer, which is
    /// occasionally what a grid-based game wants.
    Digital,
    /// Parts are analog and pass through untouched — a stick already
    /// reports its own magnitude, and normalising it would throw away
    /// how far it is actually pushed.
    Analog,
}

/// One control feeding an action.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Binding {
    /// What it reads, and its role among its neighbours.
    pub role: Role,
    /// Applied in order, on the binding and nowhere else.
    pub processors: Vec<Processor>,
}

/// A binding's place in the list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Role {
    /// Feeds the action on its own.
    Whole(ControlPath),
    /// Declares a composite. Reads no control itself — the entries after
    /// it, up to the next head, are its parts.
    CompositeHead(Composite),
    /// A named part of the composite most recently declared above it.
    Part { name: PartName, path: ControlPath },
}

impl Binding {
    /// A plain binding with no processors.
    pub fn to(path: ControlPath) -> Self {
        Self {
            role: Role::Whole(path),
            processors: Vec::new(),
        }
    }

    /// The head of a composite.
    pub fn composite(composite: Composite) -> Self {
        Self {
            role: Role::CompositeHead(composite),
            processors: Vec::new(),
        }
    }

    /// A named part of the composite above it.
    pub fn part(name: PartName, path: ControlPath) -> Self {
        Self {
            role: Role::Part { name, path },
            processors: Vec::new(),
        }
    }

    /// Adds a processor, applied after any already on this binding.
    pub fn with(mut self, processor: Processor) -> Self {
        self.processors.push(processor);
        self
    }

    /// The control this binding reads, if it reads one. A composite head
    /// does not.
    pub fn path(&self) -> Option<ControlPath> {
        match &self.role {
            Role::Whole(path) => Some(*path),
            Role::Part { path, .. } => Some(*path),
            Role::CompositeHead(_) => None,
        }
    }
}

/// Walks a flat binding list as the groups it encodes.
///
/// Yields each whole binding on its own, and each composite together with
/// the parts that follow it. Parts before any head are skipped: that is a
/// malformed list, and dropping them is better than attributing them to a
/// composite the author did not write.
pub fn groups(bindings: &[Binding]) -> Vec<Group<'_>> {
    let mut out: Vec<Group<'_>> = Vec::new();
    let mut index = 0;
    while index < bindings.len() {
        match &bindings[index].role {
            Role::Whole(path) => {
                out.push(Group::Single {
                    path: *path,
                    binding: &bindings[index],
                });
                index += 1;
            }
            Role::CompositeHead(composite) => {
                let start = index + 1;
                let mut end = start;
                while end < bindings.len() && matches!(bindings[end].role, Role::Part { .. }) {
                    end += 1;
                }
                out.push(Group::Composite {
                    composite: *composite,
                    head: &bindings[index],
                    parts: &bindings[start..end],
                });
                index = end;
            }
            // A part with no head above it. Nothing sensible to do.
            Role::Part { .. } => index += 1,
        }
    }
    out
}

/// Which entries belong with the one at `index` — itself, plus the parts
/// underneath when it is a composite head.
///
/// The same walk [`groups`] does, exposed so that removing a composite
/// and evaluating one cannot disagree about where it ends. They did:
/// deleting a head left its parts behind, and since `groups` skips a part
/// with no head above it, they became rows that were saved to the file
/// and read by nothing.
pub fn group_range(bindings: &[Binding], index: usize) -> std::ops::Range<usize> {
    if index >= bindings.len() {
        return index..index;
    }
    let mut end = index + 1;
    if matches!(bindings[index].role, Role::CompositeHead(_)) {
        while end < bindings.len() && matches!(bindings[end].role, Role::Part { .. }) {
            end += 1;
        }
    }
    index..end
}

/// A binding, or a composite and its parts, as one thing to evaluate.
#[derive(Debug)]
pub enum Group<'a> {
    Single {
        path: ControlPath,
        binding: &'a Binding,
    },
    Composite {
        composite: Composite,
        head: &'a Binding,
        parts: &'a [Binding],
    },
}

#[cfg(test)]
mod tests;
