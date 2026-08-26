//! Wraps a packaged editor into a single `.AppImage`.
//!
//! ```sh
//! cargo build --release -p kooch_editor
//! cargo run --release --features editor --example package_editor -- dist/
//! cargo run --release --features editor --example package_appimage -- dist/
//! ```
//!
//! # Why the editor and not a game
//!
//! An AppImage is one file that runs on any distribution without
//! installing anything, and it carries its own icon and name. That is
//! what an *editor* wants: it is installed, it checks what is missing,
//! and it materialises the engine for projects (#754).
//!
//! A game does not want it. A game is a binary and its pack, named for
//! the platform it runs on — `game.x86_64`, `game.exe` — and wrapping
//! that in a distribution format buys nothing and costs a tool nobody
//! has.
//!
//! # What it needs, and does not install
//!
//! `appimagetool`, which is itself an AppImage. This does not download
//! it: a packaging step that fetches binaries from the internet is a
//! dependency nobody audited and a build that breaks without a network.
//! It looks in the usual places and, when it finds nothing, prints the
//! one command that fixes it.

use std::path::{Path, PathBuf};

fn main() {
    let Some(dist) = std::env::args().nth(1).map(PathBuf::from) else {
        eprintln!("usage: package_appimage <dir-produced-by-package_editor>");
        std::process::exit(2);
    };
    let binary = dist.join("kooch_editor");
    if !binary.is_file() {
        eprintln!(
            "no editor at {} — run package_editor first:\n  \
             cargo run --release --features editor --example package_editor -- {}",
            binary.display(),
            dist.display(),
        );
        std::process::exit(1);
    }

    let Some(tool) = appimagetool() else {
        eprintln!("{}", MISSING_TOOL);
        std::process::exit(1);
    };

    // 🔴 An AppDir beside the payload rather than in place: appimagetool
    // packs the whole directory it is given, and handing it `dist/` would
    // put `AppRun` and the desktop entry inside what the next run
    // packages again.
    let appdir = dist.with_extension("AppDir");
    let _ = std::fs::remove_dir_all(&appdir);
    let usr = appdir.join("usr/bin");
    std::fs::create_dir_all(&usr).expect("create AppDir");

    // Everything package_editor produced moves in: the binary, the
    // engine source it materialises for projects, and its own assets.
    for entry in ["kooch_editor", "engine", "assets"] {
        let from = dist.join(entry);
        if from.is_dir() {
            copy_dir(&from, &usr.join(entry));
        } else if from.is_file() {
            std::fs::copy(&from, usr.join(entry)).expect("copy into AppDir");
        }
    }
    make_executable(&usr.join("kooch_editor"));

    write(&appdir.join("AppRun"), APPRUN);
    make_executable(&appdir.join("AppRun"));
    write(&appdir.join("kooch.desktop"), DESKTOP);
    // Both the name the desktop entry points at and the top-level icon
    // appimagetool looks for. It wants a file called exactly `.DirIcon`,
    // and a `<Icon>.png` beside the entry.
    let icon = repo_root().join("docs/brand/logo_hi.png");
    std::fs::copy(&icon, appdir.join("kooch.png")).expect("copy icon");
    std::fs::copy(&icon, appdir.join(".DirIcon")).expect("copy .DirIcon");

    let out = dist.with_extension("AppImage");
    let status = std::process::Command::new(&tool)
        .arg(&appdir)
        .arg(&out)
        // Without a signature and without update information: both are
        // release concerns, and a packaging step that needs a key to
        // produce anything cannot be run by whoever is just trying it.
        .env("ARCH", std::env::consts::ARCH)
        .status();

    match status {
        Ok(status) if status.success() => {
            let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
            println!(
                "packaged {} ({:.1} MB)",
                out.display(),
                size as f64 / 1_048_576.0,
            );
        }
        Ok(status) => {
            eprintln!("appimagetool failed: {status}");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("could not run {}: {e}", tool.display());
            std::process::exit(1);
        }
    }
}

/// `appimagetool`, wherever this machine keeps it.
///
/// `~/Applications` first because that is where an immutable distribution
/// puts AppImages — there is nowhere else to install one without a
/// reboot.
fn appimagetool() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("HOME") {
        let apps = Path::new(&home).join("Applications");
        for name in [
            "appimagetool.AppImage",
            "appimagetool-x86_64.AppImage",
            "appimagetool",
        ] {
            let candidate = apps.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    std::process::Command::new("which")
        .arg("appimagetool")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
}

const MISSING_TOOL: &str = "\
appimagetool was not found.

It is itself an AppImage, so it installs nothing and needs no reboot:

  mkdir -p ~/Applications
  curl -L -o ~/Applications/appimagetool.AppImage \\
    https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
  chmod +x ~/Applications/appimagetool.AppImage

Then run this again.";

/// What the AppImage runs.
///
/// 🔴 `cd` into the payload before exec. The editor resolves its own
/// assets by walking up from the executable, and materialises the engine
/// from `engine/` beside it — both are relative to where the binary sits,
/// not to where it was launched from.
const APPRUN: &str = "\
#!/bin/sh
# The mount point this run was given. Everything the editor reads sits
# under it, and it is a different path every time the AppImage starts —
# which is why nothing may be written into a project from here.
HERE=$(dirname \"$(readlink -f \"$0\")\")
exec \"$HERE/usr/bin/kooch_editor\" \"$@\"
";

const DESKTOP: &str = "\
[Desktop Entry]
Type=Application
Name=Kóoch
Comment=Game engine
Exec=kooch_editor
Icon=kooch
Categories=Development;IDE;
Terminal=false
";

fn write(path: &Path, contents: &str) {
    std::fs::write(path, contents).expect("write AppDir file");
}

fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
    }
    #[cfg(not(unix))]
    let _ = path;
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create dir");
    for entry in std::fs::read_dir(from).expect("read dir") {
        let entry = entry.expect("dir entry");
        let (src, dst) = (entry.path(), to.join(entry.file_name()));
        if src.is_dir() {
            copy_dir(&src, &dst);
        } else {
            std::fs::copy(&src, &dst).expect("copy file");
        }
    }
}
