//! The mingw runtime a cross-compiled Windows build has to carry.
//!
//! # Why a Windows build needs files from a Linux machine
//!
//! `meshopt` is C++, so the executable links against mingw's C++ standard
//! library — and that library is a **DLL**, not a static archive:
//!
//! ```text
//! roll_a_ball.exe  →  libstdc++-6.dll  →  libgcc_s_seh-1.dll
//!                                      →  libwinpthread-1.dll
//! ```
//!
//! None of the three exist on Windows. They ship with the *compiler*, so
//! a build that leaves them behind produces an executable that runs on
//! the machine that built it and nowhere else — and the failure is a
//! Windows dialog naming a DLL, on a handheld, with no way back to the
//! preset that caused it.
//!
//! # Why not link them statically
//!
//! Because it does not work, not because nobody tried. mingw's
//! `libstdc++.a` mixes static and dynamic symbols — pthread's among them
//! — so `-static-libstdc++` and `-static` both leave the import in place;
//! measured, twice, before reading rust-lang/rust#65911 and finding the
//! same problem open upstream. The route that does work lives in
//! `meshopt`'s own `build.rs`, which is not ours to change.
//!
//! # Why they are stripped on the way
//!
//! Fedora ships `libstdc++-6.dll` with its debug symbols: **29.7 MB**,
//! twice the size of the game. Stripped it is 2.5 MB, and the three
//! together come to under 3 MB. Nothing in a shipped build reads those
//! symbols.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::platform::Platform;

/// The DLLs a mingw-linked Windows build cannot start without.
///
/// A fixed list rather than one read back out of the executable's import
/// table: parsing `objdump` output to decide what to ship is a second
/// thing that can go wrong quietly, and this set is a property of the
/// toolchain rather than of any one game. A build that needs something
/// else fails loudly on Windows, which is the same place it would have
/// failed anyway.
const RUNTIME: [&str; 3] = [
    "libstdc++-6.dll",
    "libgcc_s_seh-1.dll",
    "libwinpthread-1.dll",
];

/// Copies the mingw runtime beside a Windows executable.
///
/// Returns what it wrote, so the build log can say it — a file that
/// appears without being mentioned is a file the author deletes.
///
/// 🔴 A missing DLL is an error, not a warning. Carrying on would produce
/// a folder that looks complete and holds a game that cannot start, and
/// whoever finds that out is holding a handheld rather than a compiler.
pub fn ship(platform: Platform, dir: &Path) -> Result<Vec<PathBuf>, String> {
    if platform != Platform::Windows {
        return Ok(Vec::new());
    }
    // Only for a build made *by* mingw. A Windows machine building for
    // itself links against its own runtime and needs none of this.
    if cfg!(target_os = "windows") {
        return Ok(Vec::new());
    }
    let Some(from) = runtime_dir() else {
        return Err(
            "cannot find the mingw runtime: `x86_64-w64-mingw32-gcc -print-sysroot` \
             gave no directory holding libstdc++-6.dll"
                .to_owned(),
        );
    };

    let mut written = Vec::new();
    for name in RUNTIME {
        let source = from.join(name);
        if !source.is_file() {
            return Err(format!(
                "the mingw runtime is incomplete: {} is missing from {}.\n\
                 A Windows build cannot start without it.",
                name,
                from.display(),
            ));
        }
        let dest = dir.join(name);
        std::fs::copy(&source, &dest).map_err(|e| format!("copying {name}: {e}"))?;
        strip(&dest);
        written.push(dest);
    }
    Ok(written)
}

/// Where this machine's mingw keeps its DLLs.
///
/// Asked of the toolchain rather than hardcoded per distribution:
/// Fedora puts them under a sysroot, Debian and Arch elsewhere, and a
/// list of guesses would go stale silently on whichever one nobody
/// tested.
///
/// ⚠️ `-print-file-name` is the obvious call and the wrong one — it
/// searches the *library* path, where the `.dll.a` import stubs live,
/// not the `bin` directory holding the DLLs themselves. It answers with
/// the name it was given, which reads like success.
fn runtime_dir() -> Option<PathBuf> {
    let sysroot = Command::new("x86_64-w64-mingw32-gcc")
        .arg("-print-sysroot")
        .output()
        .ok()?;
    let sysroot = String::from_utf8_lossy(&sysroot.stdout).trim().to_owned();
    let candidates = [
        // Fedora: <sysroot>/mingw/bin
        (!sysroot.is_empty()).then(|| Path::new(&sysroot).join("mingw/bin")),
        // Debian and Ubuntu keep them on the library path instead.
        Some(PathBuf::from("/usr/lib/gcc/x86_64-w64-mingw32/dll")),
        Some(PathBuf::from("/usr/x86_64-w64-mingw32/lib")),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|dir| dir.join(RUNTIME[0]).is_file())
}

/// Drops the debug symbols from a copied DLL.
///
/// Best effort: a build that ships a 29.7 MB library is worse than one
/// that ships a 2.5 MB one and better than one that ships none, so a
/// missing `strip` is not a reason to fail.
fn strip(dll: &Path) {
    let _ = Command::new("x86_64-w64-mingw32-strip")
        .arg("--strip-unneeded")
        .arg(dll)
        .status();
}

#[cfg(test)]
mod mingw_tests;
