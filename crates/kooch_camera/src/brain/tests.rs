use super::*;

#[test]
fn a_fresh_brain_drives() {
    // Added to a camera, it is there to be driven: a brain that had to be switched on after being
    // added would read as broken.
    assert!(CameraBrain::default().enabled);
}
