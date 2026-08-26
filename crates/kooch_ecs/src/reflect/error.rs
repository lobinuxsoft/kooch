use std::fmt;

use super::field::FieldKind;

/// Errors that can occur during reflection operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReflectError {
    /// The requested field name does not exist on this component.
    FieldNotFound(String),
    /// The provided value type does not match the field's expected type.
    TypeMismatch {
        field: String,
        expected: FieldKind,
        got: FieldKind,
    },
    /// The component does not exist for the given entity.
    ComponentNotFound,
    /// The storage does not support mutable access (e.g. GPU from CPU).
    ReadOnly,
    /// An entity reference reached a live component without being
    /// resolved to a handle first.
    ///
    /// A scene file stores references as
    /// [`EntityRef::Persistent`](super::EntityRef::Persistent); the load
    /// path's remapping pass turns them into handles once the target
    /// entities exist. Seeing one here means that pass did not run, and
    /// storing it would leave a component pointing nowhere.
    UnresolvedEntityRef { field: String },
    /// The value has the field's kind but not a form the field accepts —
    /// a `String` field that stores a parsed type, given text that does
    /// not parse.
    ///
    /// Distinct from [`Self::TypeMismatch`], which is about the kind:
    /// reporting a bad GUID as "expected String, got String" would say
    /// nothing about what is actually wrong.
    InvalidValue {
        field: String,
        expected: &'static str,
    },
}

impl fmt::Display for ReflectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FieldNotFound(name) => write!(f, "field not found: {name}"),
            Self::UnresolvedEntityRef { field } => write!(
                f,
                "field {field} was given an unresolved entity reference; \
                 the scene load pass must remap it to a live handle first",
            ),
            Self::TypeMismatch {
                field,
                expected,
                got,
            } => write!(
                f,
                "type mismatch on field '{field}': expected {expected:?}, got {got:?}"
            ),
            Self::InvalidValue { field, expected } => {
                write!(f, "field '{field}' cannot be read as {expected}")
            }
            Self::ComponentNotFound => write!(f, "component not found for entity"),
            Self::ReadOnly => write!(f, "storage is read-only from CPU"),
        }
    }
}

impl std::error::Error for ReflectError {}
