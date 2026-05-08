//! Small type-introspection helpers shared by the derive macro modules.

use syn::Type;

/// Extracts the last path segment identifier from a type.
/// e.g. `glam::Vec3` → "Vec3", `f32` → "f32".
pub(crate) fn last_type_segment(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(type_path) => {
            type_path.path.segments.last().map(|s| s.ident.to_string())
        }
        _ => None,
    }
}
