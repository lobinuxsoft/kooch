//! Small type-introspection helpers shared by the derive macro modules.

use syn::Type;

/// Extracts the last path segment identifier from a type.
/// e.g. `glam::Vec3` → "Vec3", `f32` → "f32".
pub(crate) fn last_type_segment(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(type_path) => type_path.path.segments.last().map(|s| s.ident.to_string()),
        _ => None,
    }
}

/// The `T` of an `Option<T>`, or `None` for any other type.
pub(crate) fn option_inner(ty: &Type) -> Option<&Type> {
    let Type::Path(type_path) = ty else {
        return None;
    };
    let segment = type_path.path.segments.last()?;
    if segment.ident != "Option" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    args.args.iter().find_map(|arg| match arg {
        syn::GenericArgument::Type(inner) => Some(inner),
        _ => None,
    })
}

/// Whether a type names `Entity`, bare or behind a path.
pub(crate) fn is_entity(ty: &Type) -> bool {
    last_type_segment(ty).is_some_and(|ident| ident == "Entity")
}

/// Whether a type names `EntityRef`, bare or behind a path.
pub(crate) fn is_entity_ref(ty: &Type) -> bool {
    last_type_segment(ty).is_some_and(|ident| ident == "EntityRef")
}
