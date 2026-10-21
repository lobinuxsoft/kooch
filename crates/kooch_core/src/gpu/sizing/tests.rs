use super::grown;

/// A pool that grows by one must not reallocate on the next frame that grows by one.
#[test]
fn growth_never_tracks_demand() {
    assert_eq!(grown(1025, 1024), 2048);
    assert_eq!(grown(2049, 2048), 4096);
}

/// From nothing the ask decides, there being no capacity to double.
#[test]
fn a_first_ask_fits() {
    assert_eq!(grown(3, 0), 4);
    assert_eq!(grown(8726, 256), 16384);
}
