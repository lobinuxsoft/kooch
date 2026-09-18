//! Lists of reflected structs (#1209): what the derive's generated code calls, so each list field
//! expands to a few calls instead of a copy of this logic.

use super::{FieldKind, Reflect, ReflectError, ReflectValue};

/// `item` as a [`ReflectValue::Struct`], its fields in declaration order.
pub fn struct_value<T: Reflect>(item: &T) -> ReflectValue {
    ReflectValue::Struct(
        item.reflect_fields()
            .iter()
            .filter_map(|meta| Some((meta.name.to_owned(), item.reflect_get(meta.name)?)))
            .collect(),
    )
}

/// The list value of `items`, with a default `T` as what a new item starts as.
pub fn list_value<T: Reflect + Default>(items: &[T]) -> ReflectValue {
    ReflectValue::List {
        items: items.iter().map(struct_value).collect(),
        element: Box::new(struct_value(&T::default())),
    }
}

/// A list field set from `value`.
///
/// 🔴 `bare` names the field a plain value fills: what a list of assets wrote to disk before its
/// items became structs. A single value is also accepted as a one-item list, for the field's
/// shape before it was a list at all.
pub fn list_from<T: Reflect + Default>(
    value: ReflectValue,
    bare: Option<&str>,
    field: &str,
) -> Result<Vec<T>, ReflectError> {
    match value {
        ReflectValue::List { items, .. } => items
            .into_iter()
            .map(|item| struct_from(item, bare, field))
            .collect(),
        single if bare.is_some() => Ok(vec![struct_from(single, bare, field)?]),
        other => Err(ReflectError::TypeMismatch {
            field: field.into(),
            expected: FieldKind::List,
            got: other.kind(),
        }),
    }
}

/// One item, starting from `T::default()`. A field the struct no longer has is skipped, so a scene
/// written by a newer or older layout still loads.
fn struct_from<T: Reflect + Default>(
    value: ReflectValue,
    bare: Option<&str>,
    field: &str,
) -> Result<T, ReflectError> {
    let mut item = T::default();
    match (value, bare) {
        (ReflectValue::Struct(fields), _) => {
            for (name, value) in fields {
                match item.reflect_set(&name, value) {
                    Ok(()) | Err(ReflectError::FieldNotFound(_)) => {}
                    Err(error) => return Err(error),
                }
            }
        }
        (plain, Some(bare)) => item.reflect_set(bare, plain)?,
        (other, None) => {
            return Err(ReflectError::TypeMismatch {
                field: field.into(),
                expected: FieldKind::Nested,
                got: other.kind(),
            });
        }
    }
    Ok(item)
}

#[cfg(test)]
mod tests;
