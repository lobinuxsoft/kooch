//! Maps Rust types to `FieldKind` / `ReflectValue` variants.

use syn::Type;

use crate::util::last_type_segment;

/// Maps a Rust type to (FieldKind variant name, type_name string, needs_clone).
pub(crate) fn type_mapping(ty: &Type) -> Option<(&'static str, &'static str, bool)> {
    let ident = last_type_segment(ty)?;
    match ident.as_str() {
        "f32" => Some(("F32", "f32", false)),
        "f64" => Some(("F64", "f64", false)),
        "u8" => Some(("U8", "u8", false)),
        "u16" => Some(("U16", "u16", false)),
        "u32" => Some(("U32", "u32", false)),
        "u64" => Some(("U64", "u64", false)),
        "i8" => Some(("I8", "i8", false)),
        "i16" => Some(("I16", "i16", false)),
        "i32" => Some(("I32", "i32", false)),
        "i64" => Some(("I64", "i64", false)),
        "bool" => Some(("Bool", "bool", false)),
        "String" => Some(("String", "String", true)),
        "Vec2" => Some(("Vec2", "Vec2", false)),
        "Vec3" => Some(("Vec3", "Vec3", false)),
        "Vec4" => Some(("Vec4", "Vec4", false)),
        "Quat" => Some(("Quat", "Quat", false)),
        "Mat4" => Some(("Mat4", "Mat4", false)),
        _ => None,
    }
}
