//! 🔴 The safety net [`COPY`](super::COPY) has needed since it was
//! written, and that two comments claimed already existed.
//!
//! `COPY` is an allowlist, so its failure mode is omitting something the
//! build needs — and that costs a full compile to find, with an error
//! naming a missing file and nothing about vendoring. `templates/` was
//! missed exactly that way.
//!
//! Anything the engine `include_str!`s or `include_bytes!`s is compiled
//! *into* it, so a vendored copy missing one does not build at all. That
//! is a property of the source, readable from the source, and this reads
//! it — rather than trusting a list somebody maintains by hand.

use super::*;

/// Every path reached by `include_str!` / `include_bytes!`, resolved
/// against the file that names it, relative to the engine root.
///
/// 🔴 Scans **what the vendor walk visits**, not the repo. Two reasons,
/// and the first cost a red test to learn: this file talks about
/// `include_str!` in order to look for it, so a scan over the repo finds
/// its own source. And test files do not travel, so a macro in one is not
/// a file the copy needs.
///
/// Only literal paths: a macro fed a `concat!` is not something a scan
/// can resolve, and there are none.
fn included_paths(repo: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    super::copy::walk_engine(repo, &mut |rel, abs| {
        if abs.extension().is_none_or(|e| e != "rs") {
            return Ok(());
        }
        let text = fs::read_to_string(abs).map_err(VendorError::Io)?;
        let owner = rel.parent().unwrap_or(Path::new(""));
        found.extend(includes_in(&text).filter_map(|target| normalise(&owner.join(target))));
        Ok(())
    })
    .expect("the repo is the engine");
    found.sort();
    found.dedup();
    found
}

/// The literal path in each `include_str!("…")` / `include_bytes!("…")`.
///
/// Commented-out occurrences are skipped: the engine explains this very
/// mechanism in prose, and a doc comment quoting `include_str!` is not a
/// file anything compiles in.
fn includes_in(text: &str) -> impl Iterator<Item = &str> {
    text.match_indices("include_").filter_map(move |(at, _)| {
        if in_a_comment(text, at) {
            return None;
        }
        let rest = text[at..]
            .strip_prefix("include_str!")
            .or_else(|| text[at..].strip_prefix("include_bytes!"))?;
        // The literal may sit on the next line — several do.
        let open = rest.find('"')?;
        // Nothing but whitespace, the bracket and the quote between the
        // macro and its argument, or this matched something else.
        if rest[..open].contains(|c: char| !c.is_whitespace() && c != '(' && c != '[') {
            return None;
        }
        let rest = &rest[open + 1..];
        rest.find('"').map(|close| &rest[..close])
    })
}

/// Whether `at` sits after a `//` on its own line.
fn in_a_comment(text: &str, at: usize) -> bool {
    let line_start = text[..at].rfind('\n').map_or(0, |n| n + 1);
    text[line_start..at].contains("//")
}

/// `a/b/../c` → `a/c`.
///
/// `Path::components` does not resolve `..` — it keeps it — and every
/// one of these paths climbs out of its own directory.
///
/// `None` for a path that climbs past the engine root, which cannot be
/// vendored and is not this test's business to report.
fn normalise(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                out.pop().then_some(())?;
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    Some(out)
}

/// The test the doc comments have been promising. Vendors the real
/// engine and asserts every compiled-in file arrived.
///
/// A fixture would prove the allowlist matches the fixture. This reads
/// what the engine actually reaches for.
#[test]
fn vendored_includes_all_resolve() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate lives two levels under the repo root")
        .to_path_buf();
    let dir = std::env::temp_dir().join("kooch_vendor_reach");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    let reached = included_paths(&repo);
    assert!(
        reached.len() > 20,
        "the scan found {} includes, which means it is not scanning — \
         the engine has dozens of shaders alone",
        reached.len(),
    );

    let dest = vendor_engine(&dir, &repo).expect("vendors the real engine");
    let missing: Vec<String> = reached
        .iter()
        .filter(|rel| !dest.join(rel).exists())
        .map(|rel| rel.display().to_string())
        .collect();

    let _ = fs::remove_dir_all(&dir);
    assert!(
        missing.is_empty(),
        "the engine compiles these in, and a vendored copy has none of them — \
         a project would fail to build with an error that says nothing about \
         vendoring:\n  {}\n\nAdd what is needed to `COPY`.",
        missing.join("\n  "),
    );
}

/// The scan has to see a path that climbs out of its crate, because that
/// is the only kind that `COPY` can get wrong — anything under `crates/`
/// travels no matter what the list says.
#[test]
fn the_scan_resolves_a_path_out_of_its_crate() {
    let owner = Path::new("crates/kooch_editor_core/src/actions");
    assert_eq!(
        normalise(&owner.join("../../../../templates/component.rs.tmpl")),
        Some(PathBuf::from("templates/component.rs.tmpl")),
    );
}

/// A doc comment quoting the macro is prose. The engine's own vendoring
/// module does exactly this, and reading it as a requirement made the
/// first run of this test demand a file nothing compiles in.
#[test]
fn the_scan_ignores_a_commented_out_include() {
    let text = "// The facade does include_str!(\"../LICENSE.md\") so it cannot be dropped.";
    assert_eq!(includes_in(text).count(), 0);
}

/// Several of the engine's includes put the literal on the line after
/// the macro, and a scan that only read the same line would silently
/// cover fewer files than it appears to.
#[test]
fn the_scan_reads_a_literal_on_the_next_line() {
    let text = "const S: &str = include_str!(\n    \"../shaders/a.wgsl\"\n);";
    assert_eq!(
        includes_in(text).collect::<Vec<_>>(),
        vec!["../shaders/a.wgsl"],
    );
}
