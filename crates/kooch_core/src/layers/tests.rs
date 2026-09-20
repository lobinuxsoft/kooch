//! The layer table's own rules: what an unnamed bit reads as, and what a file round-trips (#1218).
use super::*;

#[test]
fn an_unnamed_layer_reads_as_its_number() {
    let names = LayerNames::default();
    assert_eq!(names.label(0), "Default");
    assert_eq!(names.label(7), "Layer 7");
    assert_eq!(names.labels().len(), LAYER_COUNT);
}

/// A name that is only spaces is not a name: the box would be unidentifiable.
#[test]
fn a_blank_name_reads_as_its_number() {
    let mut names = LayerNames::default();
    names.set(3, "   ");
    assert_eq!(names.label(3), "Layer 3");
    names.set(3, "Water");
    assert_eq!(names.label(3), "Water");
}

#[test]
fn a_layer_past_the_table_is_refused() {
    let mut names = LayerNames::default();
    names.set(LAYER_COUNT, "Nowhere");
    assert_eq!(names.names.len(), 1);
}

#[test]
fn a_file_round_trips() {
    let mut names = LayerNames::default();
    names.set(2, "Water");
    let text = ron::to_string(&names).unwrap();
    assert_eq!(ron::from_str::<LayerNames>(&text).unwrap(), names);
}
