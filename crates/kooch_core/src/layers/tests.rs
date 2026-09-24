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

/// A project that never opened the matrix keeps what it had: everything meets everything.
#[test]
fn no_table_collides_with_everything() {
    let names = LayerNames::default();
    assert!(names.collide(0, 0) && names.collide(3, 7));
    assert_eq!(names.meets(2), u32::MAX);
}

/// A relationship is between two layers, so it is written both ways or it disagrees with itself.
#[test]
fn a_pair_is_symmetric() {
    let mut names = LayerNames::default();
    names.set_collide(1, 2, false);
    assert!(!names.collide(1, 2), "1 still meets 2");
    assert!(
        !names.collide(2, 1),
        "the other half of the table was left alone"
    );
    assert!(
        names.collide(1, 1) && names.collide(0, 2),
        "it turned off more than the pair"
    );
    assert_eq!(names.meets(1) & (1 << 2), 0);
}

#[test]
fn a_pair_turned_off_can_come_back() {
    let mut names = LayerNames::default();
    names.set_collide(0, 5, false);
    names.set_collide(0, 5, true);
    assert!(names.collide(0, 5) && names.collide(5, 0));
}

/// 🔴 The table has to survive the file: a matrix that writes and reads back as nothing leaves the
/// editor showing ticks the project no longer has.
#[test]
fn a_matrix_survives_the_file() {
    let mut names = LayerNames::default();
    names.set(1, "Player");
    names.set_collide(1, 2, false);
    let text = ron::ser::to_string_pretty(&names, ron::ser::PrettyConfig::default())
        .expect("it serialises");
    let read: LayerNames = ron::from_str(&text).expect("it parses");
    assert_eq!(read, names);
    assert!(!read.collide(1, 2), "the pair came back on");
}
