//! Round trips, and the properties the format is supposed to have.

use std::io::Cursor;

use crate::{Pack, PackError, PackKey, PackWriter};

/// A pack in memory, from `(name, bytes)` pairs.
fn pack(key: &PackKey, files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = PackWriter::new(Cursor::new(Vec::new()), key).unwrap();
    for (name, bytes) in files {
        writer.add(name, bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn open(bytes: &[u8], key: &PackKey) -> Result<Pack<Cursor<Vec<u8>>>, PackError> {
    Pack::from_reader(Cursor::new(bytes.to_vec()), key)
}

#[test]
fn what_goes_in_comes_out() {
    let key = PackKey::generate();
    let bytes = pack(
        &key,
        &[
            ("scenes/level.scene", b"(entities: [])"),
            ("meshes/rock.glb", &[0u8, 1, 2, 3, 255]),
            ("", b"a nameless entry is still an entry"),
        ],
    );

    let mut p = open(&bytes, &key).unwrap();
    assert_eq!(p.read("scenes/level.scene").unwrap(), b"(entities: [])");
    assert_eq!(p.read("meshes/rock.glb").unwrap(), vec![0u8, 1, 2, 3, 255]);
    assert_eq!(p.entries().len(), 3);
}

#[test]
fn an_empty_file_survives() {
    let key = PackKey::generate();
    let bytes = pack(&key, &[("empty.txt", b"")]);

    let mut p = open(&bytes, &key).unwrap();
    assert_eq!(p.read("empty.txt").unwrap(), Vec::<u8>::new());
}

#[test]
fn an_empty_pack_is_readable() {
    let key = PackKey::generate();
    let bytes = pack(&key, &[]);

    let p = open(&bytes, &key).unwrap();
    assert!(p.entries().is_empty());
}

/// 🔴 The point of the whole thing. Another key must not read it, and the
/// failure has to be an error rather than rubbish that looks like data.
#[test]
fn another_key_reads_nothing() {
    let bytes = pack(&PackKey::generate(), &[("a.txt", b"secret")]);

    assert!(matches!(
        open(&bytes, &PackKey::generate()),
        Err(PackError::Corrupt),
    ));
}

/// AES-GCM is authenticated, so a modified pack fails loudly instead of
/// handing a loader plausible rubbish that surfaces as a crash later.
#[test]
fn a_tampered_payload_is_refused() {
    let key = PackKey::generate();
    let mut bytes = pack(&key, &[("a.txt", b"the original bytes, at some length")]);

    // A byte in the payload region, past the header.
    let at = crate::write::HEADER_LEN + 4;
    bytes[at] ^= 0xff;

    let mut p = open(&bytes, &key).unwrap();
    assert!(matches!(p.read("a.txt"), Err(PackError::Corrupt)));
}

/// The index is sealed too, so the list of file names — most of what a
/// game is about — is not readable from outside, and not editable either.
#[test]
fn a_tampered_index_is_refused() {
    let key = PackKey::generate();
    let mut bytes = pack(&key, &[("a.txt", b"x")]);

    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;

    assert!(matches!(open(&bytes, &key), Err(PackError::Corrupt)));
}

/// Names are in the index and the index is encrypted, so nothing about
/// the contents leaks from the raw file.
#[test]
fn no_name_is_readable_in_the_raw_bytes() {
    let key = PackKey::generate();
    let bytes = pack(&key, &[("levels/secret_ending.scene", b"spoilers")]);

    let haystack = String::from_utf8_lossy(&bytes);
    assert!(!haystack.contains("secret_ending"));
    assert!(!haystack.contains("spoilers"));
}

#[test]
fn something_else_entirely_is_not_a_pack() {
    let key = PackKey::generate();
    assert!(matches!(
        open(b"just a text file, honestly", &key),
        Err(PackError::NotAPack),
    ));
    assert!(matches!(open(b"", &key), Err(PackError::NotAPack)));
}

/// A pack from a newer editor is refused by name rather than parsed as
/// though the layout had not moved.
#[test]
fn a_newer_format_is_refused() {
    let key = PackKey::generate();
    let mut bytes = pack(&key, &[("a.txt", b"x")]);
    bytes[8..10].copy_from_slice(&99u16.to_le_bytes());

    assert!(matches!(open(&bytes, &key), Err(PackError::Version(99))));
}

#[test]
fn a_missing_entry_says_so() {
    let key = PackKey::generate();
    let bytes = pack(&key, &[("a.txt", b"x")]);

    let mut p = open(&bytes, &key).unwrap();
    assert!(matches!(p.read("b.txt"), Err(PackError::NotFound(_))));
    assert!(p.contains("a.txt"));
    assert!(!p.contains("b.txt"));
}

/// Two files with the same name means the caller merged trees that
/// collide. Refused at pack time, where the two paths are still known —
/// at read time one of them has simply vanished.
#[test]
fn a_duplicate_name_is_refused() {
    let key = PackKey::generate();
    let mut writer = PackWriter::new(Cursor::new(Vec::new()), &key).unwrap();
    writer.add("a.txt", b"first").unwrap();

    assert!(writer.add("a.txt", b"second").is_err());
}

/// A pack built on Windows has to read on Linux, so separators are
/// normalised going in and coming out.
#[test]
fn separators_are_normalised() {
    let key = PackKey::generate();
    let bytes = pack(&key, &[("meshes\\props\\rock.glb", b"mesh")]);

    let mut p = open(&bytes, &key).unwrap();
    assert_eq!(p.entries()[0].name, "meshes/props/rock.glb");
    assert_eq!(p.read("meshes/props/rock.glb").unwrap(), b"mesh");
    assert_eq!(p.read("meshes\\props\\rock.glb").unwrap(), b"mesh");
    assert_eq!(p.read("/meshes/props/rock.glb").unwrap(), b"mesh");
}

/// Same inputs, same bytes. A build that differs from itself is one
/// nobody can check — except for the nonces, which must differ or the
/// encryption is broken.
#[test]
fn the_index_order_is_stable() {
    let key = PackKey::generate();
    let a = pack(&key, &[("b.txt", b"2"), ("a.txt", b"1"), ("c.txt", b"3")]);
    let b = pack(&key, &[("c.txt", b"3"), ("a.txt", b"1"), ("b.txt", b"2")]);

    let names = |bytes: &[u8]| -> Vec<String> {
        open(bytes, &key)
            .unwrap()
            .entries()
            .iter()
            .map(|e| e.name.clone())
            .collect()
    };
    assert_eq!(names(&a), vec!["a.txt", "b.txt", "c.txt"]);
    assert_eq!(names(&a), names(&b));
}

/// 🔴 A nonce reused across entries under one key breaks AES-GCM
/// outright — it is not a weakening, it leaks the plaintext difference.
#[test]
fn every_entry_gets_its_own_nonce() {
    let key = PackKey::generate();
    let bytes = pack(
        &key,
        &[("a.txt", b"same"), ("b.txt", b"same"), ("c.txt", b"same")],
    );

    let p = open(&bytes, &key).unwrap();
    let mut nonces: Vec<_> = p.entries().iter().map(|e| e.nonce).collect();
    nonces.sort();
    nonces.dedup();
    assert_eq!(nonces.len(), 3, "a nonce was reused");
}

/// Text compresses; a `.png` is left alone because zstd over it costs
/// time to make it slightly bigger.
#[test]
fn already_compressed_formats_are_left_alone() {
    let key = PackKey::generate();
    let text = vec![b'a'; 8192];
    let bytes = pack(&key, &[("a.txt", &text), ("b.png", &text)]);

    let p = open(&bytes, &key).unwrap();
    let entry = |name: &str| p.entries().iter().find(|e| e.name == name).unwrap();
    assert!(entry("a.txt").compressed);
    assert!(!entry("b.png").compressed);
    assert!(
        entry("a.txt").stored_len < entry("b.png").stored_len,
        "the compressible one is not smaller",
    );
}

/// The reason to compress at all.
#[test]
fn compressible_data_gets_smaller() {
    let key = PackKey::generate();
    let text = vec![b'x'; 64 * 1024];
    let bytes = pack(&key, &[("a.txt", &text)]);

    assert!(
        bytes.len() < text.len() / 10,
        "64 KiB of one byte packed to {} bytes",
        bytes.len(),
    );
}

/// What the editor's "verify this build" runs.
#[test]
fn verify_reads_everything() {
    let key = PackKey::generate();
    let bytes = pack(&key, &[("a.txt", b"1"), ("b.txt", b"2")]);

    assert_eq!(open(&bytes, &key).unwrap().verify().unwrap(), 2);
}

#[test]
fn verify_finds_damage() {
    let key = PackKey::generate();
    let mut bytes = pack(&key, &[("a.txt", b"a longer payload to damage")]);
    let at = crate::write::HEADER_LEN + 2;
    bytes[at] ^= 0xff;

    assert!(matches!(
        open(&bytes, &key).unwrap().verify(),
        Err(PackError::Corrupt),
    ));
}

/// Big enough to cross the buffer sizes zstd works in, so the streaming
/// path is exercised rather than only the one-shot.
#[test]
fn a_large_entry_round_trips() {
    let key = PackKey::generate();
    let data: Vec<u8> = (0..2_000_000u32).map(|i| (i % 251) as u8).collect();
    let bytes = pack(&key, &[("big.bin", &data)]);

    let mut p = open(&bytes, &key).unwrap();
    assert_eq!(p.read("big.bin").unwrap(), data);
}
