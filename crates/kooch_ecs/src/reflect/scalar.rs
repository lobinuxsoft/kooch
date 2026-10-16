//! Lists of plain values: `Vec<u32>`, `Vec<Vec3>` and the rest (#1452).
//!
//! 🔴 The sibling of [`nested`](super::nested), which handles `Vec<T>` where `T` is a reflected
//! struct. A list of numbers had no way through reflection at all, so anything shaped like a mesh —
//! positions, indices, the offsets between them — had to be an asset instead of a component. That
//! constraint is what put a blockout's geometry in `.block` files, and this is what lifts it.

use glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use super::{FieldKind, ReflectError, ReflectValue};

/// A value a list can hold directly, with no fields of its own.
///
/// Implemented for exactly the types `FieldKind` already names, so a list holds what a field could
/// have held on its own.
pub trait ReflectScalar: Sized {
    /// This value as a reflected one.
    fn to_value(&self) -> ReflectValue;
    /// Reads one back, or says what it found instead.
    fn from_value(value: ReflectValue, field: &str) -> Result<Self, ReflectError>;
    /// What an empty list says it holds, so the Inspector's add button has something to add.
    fn empty() -> ReflectValue;
}

/// `impl ReflectScalar` for one type, given the `ReflectValue` variant that carries it.
macro_rules! scalar {
    ($ty:ty, $variant:ident, $kind:ident, $zero:expr) => {
        impl ReflectScalar for $ty {
            fn to_value(&self) -> ReflectValue {
                ReflectValue::$variant(self.clone())
            }

            fn from_value(value: ReflectValue, field: &str) -> Result<Self, ReflectError> {
                match value {
                    ReflectValue::$variant(held) => Ok(held),
                    other => Err(ReflectError::TypeMismatch {
                        field: field.into(),
                        expected: FieldKind::$kind,
                        got: other.kind(),
                    }),
                }
            }

            fn empty() -> ReflectValue {
                ReflectValue::$variant($zero)
            }
        }
    };
}

scalar!(f32, F32, F32, 0.0);
scalar!(f64, F64, F64, 0.0);
scalar!(u8, U8, U8, 0);
scalar!(u16, U16, U16, 0);
scalar!(u32, U32, U32, 0);
scalar!(u64, U64, U64, 0);
scalar!(i8, I8, I8, 0);
scalar!(i16, I16, I16, 0);
scalar!(i32, I32, I32, 0);
scalar!(i64, I64, I64, 0);
scalar!(bool, Bool, Bool, false);
scalar!(String, String, String, String::new());
scalar!(Vec2, Vec2, Vec2, Vec2::ZERO);
scalar!(Vec3, Vec3, Vec3, Vec3::ZERO);
scalar!(Vec4, Vec4, Vec4, Vec4::ZERO);
scalar!(Quat, Quat, Quat, Quat::IDENTITY);
scalar!(Mat4, Mat4, Mat4, Mat4::IDENTITY);

/// The list value of `items`, what the derive calls for a `Vec` of plain values.
pub fn scalar_list<T: ReflectScalar>(items: &[T]) -> ReflectValue {
    ReflectValue::List {
        items: items.iter().map(T::to_value).collect(),
        element: Box::new(T::empty()),
    }
}

/// A list field read back from `value`.
///
/// A single value is accepted as a one-item list, the way [`nested::list_from`](super::nested)
/// does: it is the same field written before it was a list.
pub fn scalar_list_from<T: ReflectScalar>(
    value: ReflectValue,
    field: &str,
) -> Result<Vec<T>, ReflectError> {
    match value {
        ReflectValue::List { items, .. } => items
            .into_iter()
            .map(|item| T::from_value(item, field))
            .collect(),
        single => Ok(vec![T::from_value(single, field)?]),
    }
}

#[cfg(test)]
mod tests;
