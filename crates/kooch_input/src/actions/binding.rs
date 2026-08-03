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
    Vector2 { mode: Vector2Mode },
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

/// How a 2D composite reads its parts.
///
/// Named after Unity's, because the distinction is real and hard-won.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Vector2Mode {
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
mod tests {
    use super::*;
    use crate::ids::{GamepadAxis, GamepadButton, KeyCode};

    fn wasd() -> Vec<Binding> {
        vec![
            Binding::composite(Composite::Vector2 {
                mode: Vector2Mode::DigitalNormalized,
            }),
            Binding::part(PartName::Up, ControlPath::Key(KeyCode::KeyW)),
            Binding::part(PartName::Down, ControlPath::Key(KeyCode::KeyS)),
            Binding::part(PartName::Left, ControlPath::Key(KeyCode::KeyA)),
            Binding::part(PartName::Right, ControlPath::Key(KeyCode::KeyD)),
        ]
    }

    /// The flat list has to read back as the structure it encodes.
    #[test]
    fn a_composite_gathers_the_parts_that_follow_it() {
        let mut bindings = wasd();
        bindings.push(Binding::to(ControlPath::Button(GamepadButton::South)));

        let groups = groups(&bindings);
        assert_eq!(groups.len(), 2, "expected one composite and one single");

        match &groups[0] {
            Group::Composite { parts, .. } => assert_eq!(parts.len(), 4),
            other => panic!("expected a composite, got {other:?}"),
        }
        assert!(matches!(groups[1], Group::Single { .. }));
    }

    /// Two composites on one action — keyboard and stick both driving
    /// "move" — must not bleed into each other.
    #[test]
    fn a_second_composite_starts_a_new_group() {
        let mut bindings = wasd();
        bindings.push(Binding::composite(Composite::Vector2 {
            mode: Vector2Mode::Analog,
        }));
        bindings.push(Binding::part(
            PartName::Up,
            ControlPath::Axis(GamepadAxis::LeftStickY),
        ));

        let groups = groups(&bindings);
        assert_eq!(groups.len(), 2);
        match (&groups[0], &groups[1]) {
            (Group::Composite { parts: a, .. }, Group::Composite { parts: b, .. }) => {
                assert_eq!(a.len(), 4, "the keyboard composite swallowed the stick's");
                assert_eq!(b.len(), 1);
            }
            other => panic!("expected two composites, got {other:?}"),
        }
    }

    /// A malformed list — parts with no head — must not be attributed to
    /// a composite nobody wrote.
    #[test]
    fn orphan_parts_are_dropped_rather_than_guessed_at() {
        let bindings = vec![
            Binding::part(PartName::Up, ControlPath::Key(KeyCode::KeyW)),
            Binding::to(ControlPath::Key(KeyCode::Space)),
        ];
        let groups = groups(&bindings);
        assert_eq!(groups.len(), 1);
        assert!(matches!(groups[0], Group::Single { .. }));
    }

    /// The default of a 2D composite has to be the one WASD needs, since
    /// that is what the editor will hand an author who changes nothing.
    #[test]
    fn the_defaults_are_the_ones_a_keyboard_wants() {
        assert_eq!(Vector2Mode::default(), Vector2Mode::DigitalNormalized);
        assert_eq!(
            BothHeld::default(),
            BothHeld::Neither,
            "left and right together should cancel, not pick a winner"
        );
    }

    /// The whole model has to survive a round trip, because it is going
    /// to live in a file.
    #[test]
    fn a_binding_list_round_trips() {
        let bindings = wasd();
        let encoded = ron::to_string(&bindings).expect("serialise");
        let decoded: Vec<Binding> = ron::from_str(&encoded).expect("deserialise");
        assert_eq!(decoded, bindings);
    }
}
