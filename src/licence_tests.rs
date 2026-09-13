/// 🔴 Linked as an `rlib`, the licence string is inside every shipped executable — mandatory without
/// anyone copying a file.
#[test]
fn every_binary_that_links_the_engine_carries_the_licence() {
    assert!(super::LICENSE.contains("All Rights Reserved"));
    assert!(super::LICENSE.contains("Matías Galarza"));
}
