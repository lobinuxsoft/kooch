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
}

impl fmt::Display for ReflectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FieldNotFound(name) => write!(f, "field not found: {name}"),
            Self::TypeMismatch {
                field,
                expected,
                got,
            } => write!(f, "type mismatch on field '{field}': expected {expected:?}, got {got:?}"),
            Self::ComponentNotFound => write!(f, "component not found for entity"),
            Self::ReadOnly => write!(f, "storage is read-only from CPU"),
        }
    }
}

impl std::error::Error for ReflectError {}
