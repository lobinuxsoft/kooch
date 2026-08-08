use super::*;

/// The registry has to be non-empty in a binary that links any crate
/// declaring an asset — an empty inventory reads as "no asset types"
/// and would make every check below vacuously pass.
///
/// `kooch_core` declares none of its own, so this asserts the shape
/// rather than a count: names are unique, and nothing is blank.
#[test]
fn every_registration_names_a_distinct_type() {
    let mut seen: Vec<&str> = Vec::new();
    for registration in registered_asset_types() {
        let name = (registration.type_name)();
        assert!(!name.is_empty(), "an asset registration has no type name");
        assert!(
            !seen.contains(&name),
            "{name} is registered twice; its loader would be installed \
                 twice and the second would win silently",
        );
        seen.push(name);
    }
}
