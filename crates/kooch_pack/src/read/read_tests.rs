use super::*;

/// The index is authenticated, so a short one is a bug in the writer —
/// and a shipped game must report that rather than panic on a slice.
#[test]
fn a_truncated_index_is_an_error_not_a_panic() {
    // Claims one entry, and carries two bytes of it.
    assert!(matches!(parse_index(&[4, 0], 1), Err(PackError::Corrupt),));
    assert!(matches!(parse_index(&[], 1), Err(PackError::Corrupt)));
}

/// A name length that would run past the end must not index out of
/// bounds.
#[test]
fn an_impossible_name_length_is_refused() {
    let mut bytes = u16::MAX.to_le_bytes().to_vec();
    bytes.extend_from_slice(b"short");

    assert!(matches!(parse_index(&bytes, 1), Err(PackError::Corrupt)));
}

/// Bytes that are not UTF-8 in a name field. Same reasoning: authenticated
/// input, so this is a writer bug, and it has to be an error.
#[test]
fn a_non_utf8_name_is_refused() {
    let mut bytes = 2u16.to_le_bytes().to_vec();
    bytes.extend_from_slice(&[0xff, 0xfe]);
    bytes.extend_from_slice(&[0u8; 8 + 8 + 8 + 1 + NONCE_LEN]);

    assert!(matches!(parse_index(&bytes, 1), Err(PackError::Corrupt)));
}

#[test]
fn no_entries_parses_to_nothing() {
    assert_eq!(parse_index(&[], 0).unwrap(), Vec::new());
}

/// The reader and the writer have to agree on what a name is, or a lookup
/// misses what `add` stored.
#[test]
fn lookup_normalises_the_same_way() {
    assert_eq!(normalise("a\\b.png"), "a/b.png");
    assert_eq!(normalise("/a/b.png"), "a/b.png");
}
