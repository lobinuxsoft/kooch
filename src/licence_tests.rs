/// 🔴 A game links the engine as an `rlib`, so this string is inside
/// every shipped executable. That is what makes the licence
/// mandatory in a release build without anyone having to remember
/// to ship a file next to it.
#[test]
fn every_binary_that_links_the_engine_carries_the_licence() {
    assert!(super::LICENSE.contains("All Rights Reserved"));
    assert!(super::LICENSE.contains("Matías Galarza"));
}
