use super::*;

#[test]
fn separators_and_leading_slashes_go() {
    assert_eq!(normalise("a\\b\\c.png"), "a/b/c.png");
    assert_eq!(normalise("/a/b.png"), "a/b.png");
    assert_eq!(normalise("\\a\\b.png"), "a/b.png");
    assert_eq!(normalise("a/b.png"), "a/b.png");
}

/// The list is about *contents*, not about being a known extension: text
/// formats compress well however unusual they are, and a container that
/// is already deflated does not.
#[test]
fn already_compressed_formats_are_recognised() {
    assert!(worth_compressing("scene.ron"));
    assert!(worth_compressing("mesh.glb"));
    assert!(worth_compressing("no_extension"));
    assert!(!worth_compressing("tex.png"));
    assert!(!worth_compressing("sound.ogg"));
}

/// A name that arrives shouting must not be treated as a different
/// format from the same name in lowercase.
#[test]
fn the_extension_check_ignores_case() {
    assert!(!worth_compressing("TEX.PNG"));
    assert!(!worth_compressing("Sound.Ogg"));
}

/// The header has to be exactly what the reader slices apart. Two
/// constants for one layout is how a format quietly stops parsing.
#[test]
fn the_header_is_the_size_the_reader_expects() {
    assert_eq!(HEADER_LEN, 8 + 2 + 4 + 8 + 8 + crate::NONCE_LEN);
    assert_eq!(HEADER_LEN, 42);
}
