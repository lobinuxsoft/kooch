use super::*;

/// Whatever is registered must be named distinctly, or a lookup by
/// type name returns whichever the linker happened to order first.
#[test]
fn registrations_name_distinct_types() {
    let mut seen: Vec<&str> = Vec::new();
    for registration in reflected_asset_types() {
        let name = (registration.type_name)();
        assert!(!name.is_empty());
        assert!(!seen.contains(&name), "{name} is registered twice");
        seen.push(name);
    }
}

/// An unknown type resolves to nothing rather than to the first
/// registration — the editor falls back to its "no settings" label,
/// which is honest, instead of editing the wrong asset's fields.
#[test]
fn an_unregistered_type_resolves_to_nothing() {
    assert!(reflected_asset("not::a::real::Asset").is_none());
}
