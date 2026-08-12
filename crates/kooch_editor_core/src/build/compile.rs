//! Running cargo for a build preset, and packaging what comes out (#758).
//!
//! A build takes minutes, so this is a child process polled each frame —
//! the same shape as `PlayState` and the project launcher, for the same
//! reason: the editor has to stay drawable while it happens.
//!
//! # 🔴 What is checked before cargo runs
//!
//! A missing cross-compilation target fails **ten minutes in**, with a
//! linker error that says nothing about targets. Both of the things that
//! can be known up front are checked up front, and the message names the
//! command that fixes it.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use kooch_core::Guid;
use kooch_pack::PackKey;

use super::{BuildPreset, Package, PackageError};

/// Where a build has got to.
#[derive(Debug, Clone)]
pub enum BuildStatus {
    /// cargo is running, and what it was told to build.
    ///
    /// 🔴 The configuration travels with the status rather than being
    /// read back off the selected preset. The list is editable while a
    /// build runs: selecting another row, or editing the one that is
    /// building, would otherwise silently relabel a build already in
    /// flight — and the whole reason to show this is that a four-minute
    /// compile should say what it is compiling.
    Compiling {
        /// Which preset started it, so the panel can name it.
        preset: Guid,
        /// What cargo was actually asked for: mode, target, floor.
        what: String,
    },
    /// cargo finished; the folder is being laid out.
    Packaging,
    /// Everything worked.
    Done(Package),
    Failed(String),
    /// Stopped on purpose.
    ///
    /// Separate from `Failed` because it is not one: nothing went wrong,
    /// and a red error where someone pressed a button reads as a bug.
    Cancelled,
}

/// A build in progress.
pub struct BuildJob {
    child: Option<Child>,
    output: Arc<Mutex<Vec<String>>>,
    status: BuildStatus,
    preset: BuildPreset,
    project_root: PathBuf,
    engine_root: Option<PathBuf>,
    crate_name: String,
    key: PackKey,
    /// Extensions a registered loader claims — the packaging allowlist,
    /// captured at start because the job outlives the frame that had the
    /// asset server.
    known: Vec<String>,
}

impl BuildJob {
    /// Checks what can be checked, then starts cargo.
    pub fn start(
        preset: &BuildPreset,
        preset_guid: Guid,
        project_root: &Path,
        engine_root: Option<&Path>,
        crate_name: &str,
        key: PackKey,
        known: Vec<String>,
    ) -> Result<Self, String> {
        if let Some(problem) = missing_toolchain(preset) {
            return Err(problem);
        }

        let mut command = cargo_command(preset, project_root, crate_name);
        if preset.pack_assets {
            // The shares the game reassembles its key from. Through the
            // environment, so nothing is written into the project — see
            // `kooch::shipped`.
            command.env(
                kooch_core::asset_loader::SHARES_ENV,
                kooch_core::asset_loader::shares_for_build(&key),
            );
        }
        command.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = command
            .spawn()
            .map_err(|e| format!("could not start cargo: {e}"))?;
        let output = Arc::new(Mutex::new(Vec::new()));
        capture(child.stdout.take(), &output);
        capture(child.stderr.take(), &output);

        Ok(Self {
            child: Some(child),
            output,
            status: BuildStatus::Compiling {
                preset: preset_guid,
                what: describe(preset),
            },
            preset: preset.clone(),
            project_root: project_root.to_path_buf(),
            engine_root: engine_root.map(Path::to_path_buf),
            crate_name: crate_name.to_owned(),
            key,
            known,
        })
    }

    /// Where the build is. Call once a frame.
    pub fn status(&self) -> &BuildStatus {
        &self.status
    }

    /// Lines cargo has produced, taken out of the buffer.
    pub fn drain_output(&self) -> Vec<String> {
        self.output
            .lock()
            .map(|mut buffer| std::mem::take(&mut *buffer))
            .unwrap_or_default()
    }

    /// Advances the job. Packaging happens here, on the frame cargo
    /// exits, because it is fast enough not to need its own process and
    /// slow enough to be worth a status of its own.
    pub fn poll(&mut self) {
        let Some(child) = self.child.as_mut() else {
            return;
        };
        match child.try_wait() {
            Ok(None) => {}
            Ok(Some(exit)) => {
                self.child = None;
                if !exit.success() {
                    // cargo's own words are in the log; this is the line
                    // that says which step failed — plus the cause, when
                    // it is one this editor already knows about.
                    let mut why = format!("cargo exited with {exit} — see the log");
                    if let Some(hint) = unmigrated_main(&self.project_root) {
                        why.push_str(&hint);
                    }
                    self.status = BuildStatus::Failed(why);
                    return;
                }
                self.status = BuildStatus::Packaging;
                self.package();
            }
            Err(e) => {
                self.child = None;
                self.status = BuildStatus::Failed(format!("lost track of cargo: {e}"));
            }
        }
    }

    fn package(&mut self) {
        let binary = built_binary(&self.preset, &self.project_root, &self.crate_name);
        let result = super::assemble(
            &self.preset,
            &self.known,
            &self.project_root,
            self.engine_root.as_deref(),
            &binary,
            &self.crate_name,
            &self.key,
        );
        self.status = match result {
            Ok(package) => {
                for name in &package.shadowed {
                    tracing::warn!(
                        asset = %name,
                        "a project asset replaced the engine's of the same name",
                    );
                }
                BuildStatus::Done(package)
            }
            Err(PackageError::NoBinary(path)) => BuildStatus::Failed(format!(
                "cargo succeeded but produced no executable at {} — check the \
                 preset's target triple",
                path.display(),
            )),
            Err(e) => BuildStatus::Failed(e.to_string()),
        };
    }

    /// Stops the build.
    ///
    /// ⚠️ cargo is killed, not asked. It has no "stop when convenient",
    /// and a build that keeps compiling after the button says it stopped
    /// is worse than an interrupted one — cargo recovers from a kill by
    /// redoing whatever crate it was on.
    ///
    /// Nothing is packaged, so a half-built executable never reaches the
    /// output folder: `assemble` only runs when cargo exits clean.
    pub fn cancel(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.child = None;
        self.status = BuildStatus::Cancelled;
    }
}

impl Drop for BuildJob {
    /// A build outliving the editor would keep a `target/` lock and half
    /// a `dist/` behind it.
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// The cargo invocation a preset describes.
///
/// 🔴 `--bin <crate>` names the **game**, never `<crate>_editor`. The
/// authoring binary is gated behind a feature a shipped build does not
/// enable (#558), and asking for it by accident is the one way to put the
/// editor back into a release.
pub fn cargo_command(preset: &BuildPreset, project_root: &Path, crate_name: &str) -> Command {
    let floor = preset.glibc_floor();
    let mut command = Command::new("cargo");
    // `zigbuild` is a cargo subcommand, so everything after it is the
    // ordinary `build` invocation — only the linker changes.
    command
        .arg(match floor {
            Some(_) => "zigbuild",
            None => "build",
        })
        .arg("--manifest-path")
        .arg(project_root.join("Cargo.toml"))
        .arg("--bin")
        .arg(crate_name);
    command.arg("--release");
    full_optimisation(&mut command);
    if let Some(triple) = build_triple(preset) {
        // `x86_64-unknown-linux-gnu.2.28` — zigbuild's own spelling for
        // "this target, against that glibc".
        command.arg("--target").arg(match floor {
            Some(floor) => format!("{triple}.{floor}"),
            None => triple,
        });
    }
    if floor.is_some() {
        allow_shlib_undefined(&mut command);
    }
    let features = preset.feature_list();
    if !features.is_empty() {
        command.arg("--features").arg(features.join(","));
    }
    // 🔴 mingw's gcc defaults to C23, where `false` is a keyword, and
    // GKlib declares an enum member with that name — so metis-sys fails
    // to build for Windows without this. Measured, not guessed: the
    // engine cross-compiles with it and does not without.
    if preset.target_triple.contains("windows-gnu") {
        command.env("CFLAGS_x86_64_pc_windows_gnu", "-std=gnu17");
    }
    command
}

/// The triple cargo is told to build for, or `None` for "this machine,
/// however cargo spells it".
///
/// 🔴 A glibc floor forces one even when the preset asked for the host.
/// `cargo zigbuild` has nothing to attach the version to without an
/// explicit `--target`, and without it the floor is silently ignored —
/// a build that looks like it worked and still will not start on the
/// handheld.
fn build_triple(preset: &BuildPreset) -> Option<String> {
    let triple = preset.target_triple.trim();
    if !triple.is_empty() {
        return Some(triple.to_owned());
    }
    preset.glibc_floor().and_then(|_| host_triple())
}

/// What `rustc` calls this machine.
///
/// Asked rather than assembled from `std::env::consts`: those give
/// `x86_64` and `linux` with no vendor and no ABI, and the triple has to
/// match a directory cargo will create exactly.
fn host_triple() -> Option<String> {
    let out = Command::new("rustc").arg("-vV").output().ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .map(|t| t.trim().to_owned())
}

/// Lets the link go through with symbols the *host's* shared libraries
/// need and the target's glibc does not have.
///
/// 🔴 Measured, not guessed. Linking a game against a 2.28 sysroot fails
/// on `pthread_join@GLIBC_2.34 referenced by /usr/lib64/libasound.so` —
/// symbols of the **build machine's** ALSA, which is not the one the game
/// loads. At runtime the target's `ld.so` resolves them against the
/// target's own libc, where they exist. The check is asking a question
/// about the wrong machine.
///
/// It stays narrow: only when a floor was asked for, and appended so a
/// project's own `RUSTFLAGS` survive instead of being replaced.
fn allow_shlib_undefined(command: &mut Command) {
    const FLAG: &str = "-C link-arg=-Wl,--allow-shlib-undefined";
    let flags = match std::env::var("RUSTFLAGS") {
        Ok(existing) if !existing.trim().is_empty() => format!("{existing} {FLAG}"),
        _ => FLAG.to_owned(),
    };
    command.env("RUSTFLAGS", flags);
}

/// One line saying what cargo was actually asked to produce.
///
/// The mode leads it: it is the difference between a build you hand out
/// and one that opens a listening socket, and a compile long enough to
/// walk away from should say which one it is making.
fn describe(preset: &BuildPreset) -> String {
    let mut parts = vec![preset.mode_label().to_owned()];
    parts.push(match preset.is_host() {
        true => "this machine".to_owned(),
        false => preset.target_triple.trim().to_owned(),
    });
    if let Some(floor) = preset.glibc_floor() {
        parts.push(format!("glibc {floor}+"));
    }
    if !preset.pack_assets {
        parts.push("loose assets".to_owned());
    }
    let features = preset.feature_list();
    if !features.is_empty() {
        parts.push(features.join(" "));
    }
    parts.join(", ")
}

/// Turns cargo's release profile up to what a shipped game wants: link
/// time optimisation across every crate, and one codegen unit so the
/// optimiser sees a whole crate at a time.
///
/// 🔴 **Through the environment, never the project's `Cargo.toml`.**
/// The manifest is generated once, when the project is created, so a
/// `[profile.release]` written into the template would reach new
/// projects and silently skip every one that already exists — the same
/// trap `PROFILING_FEATURE` documents. `CARGO_PROFILE_*` applies to
/// whatever project is being built.
///
/// ⚠️ It costs minutes per build, and it buys throughput on the CPU
/// side. On the OneXFly the frame is GPU-bound at 96 %, so this is not
/// the lever that moves that frame — it is what keeps the measured
/// binary and the shipped binary the same one.
///
/// A value already in the environment wins: someone who set it meant it.
fn full_optimisation(command: &mut Command) {
    for (key, value) in [
        ("CARGO_PROFILE_RELEASE_LTO", "fat"),
        ("CARGO_PROFILE_RELEASE_CODEGEN_UNITS", "1"),
    ] {
        if std::env::var_os(key).is_none() {
            command.env(key, value);
        }
    }
}

/// Where cargo leaves the executable for this preset.
pub fn built_binary(preset: &BuildPreset, project_root: &Path, crate_name: &str) -> PathBuf {
    let mut path = project_root.join("target");
    // 🔴 `build_triple`, not the preset's field: a host build with a
    // glibc floor is still given a `--target`, and cargo puts anything
    // with one in `target/<triple>/` rather than `target/`.
    if let Some(triple) = build_triple(preset) {
        path = path.join(triple);
    }
    // The name cargo writes, which is the crate's — the preset's own
    // name is what the *copy* is called.
    let produced = match preset.target_triple.contains("windows") {
        true => format!("{crate_name}.exe"),
        false => crate_name.to_owned(),
    };
    path.join(preset.profile_dir()).join(produced)
}

/// The likely cause when a build fails and `main.rs` still starts the
/// editor.
///
/// 🔴 The migration deliberately leaves an edited `main.rs` alone (#558)
/// — deleting someone's gameplay setup would be worse than doing
/// nothing — and warns when the project opens. But that warning is a
/// hundred lines above the error, in a different panel, at a different
/// time. Someone pressing Build sees a compiler error naming
/// `kooch_editor_core`, which they never wrote.
///
/// So the failure says it too, where it is being read.
fn unmigrated_main(project_root: &Path) -> Option<String> {
    let main = std::fs::read_to_string(project_root.join("src/main.rs")).ok()?;
    if !main.contains("run_editor_with") {
        return None;
    }
    Some(
        "\n\nsrc/main.rs still starts the editor, and a game build has no editor \
         in it — that is what the unresolved `kooch_editor_core` / `kooch_remote` \
         above are. Replace main.rs with the plain `App::new()` form; what it \
         used to do is already in src/editor.rs (#558)."
            .to_owned(),
    )
}

/// A reason this build cannot start, or `None`.
///
/// Only what is knowable without compiling. A missing C toolchain is not
/// — that surfaces from cargo, and guessing at it would mean refusing
/// builds that would have worked.
fn missing_toolchain(preset: &BuildPreset) -> Option<String> {
    if preset.glibc_floor().is_some()
        && let Some(problem) = missing_zig()
    {
        return Some(problem);
    }
    if preset.is_host() {
        return None;
    }
    let triple = preset.target_triple.trim();
    let installed = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .ok()?;
    let installed = String::from_utf8_lossy(&installed.stdout);
    if installed.lines().any(|line| line.trim() == triple) {
        return None;
    }
    // 🔴 The whole reason this check exists: without it cargo runs for
    // ten minutes and fails with a linker error that never says the word
    // "target".
    Some(format!(
        "the target {triple} is not installed — run:\n  rustup target add {triple}",
    ))
}

/// What a glibc floor needs and this machine has not got.
///
/// Both pieces are checked separately because they are installed
/// separately and the fix differs: `cargo-zigbuild` is a cargo
/// subcommand, `zig` is the compiler it drives.
fn missing_zig() -> Option<String> {
    let mut missing = Vec::new();
    if !on_path("cargo-zigbuild", "--version") {
        missing.push(
            "cargo-zigbuild — the cargo subcommand:\n  cargo install cargo-zigbuild".to_owned(),
        );
    }
    // ⚠️ `zig version`, not `zig --version` — zig treats the flag as an
    // unknown command and exits 1, which would report it as missing on a
    // machine that has it.
    if !on_path("zig", "version") {
        // ⚠️ Deliberately not `dnf`/`apt`: an immutable distribution has
        // no such thing, and the tarball needs no root anywhere.
        missing.push(
            "zig — the linker it drives. It is one tarball, no root needed:\n  \
             https://ziglang.org/download/ — unpack it and put the folder on PATH"
                .to_owned(),
        );
    }
    if missing.is_empty() {
        return None;
    }
    Some(format!(
        "this preset asks for a glibc floor, which needs:\n\n{}\n\nOr clear \
         `min_glibc` to link against this machine's glibc — the build will \
         then only run on systems at least as new as this one.",
        missing.join("\n\n"),
    ))
}

fn on_path(program: &str, version_arg: &str) -> bool {
    Command::new(program)
        .arg(version_arg)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Forwards a pipe into the shared buffer.
fn capture(stream: Option<impl std::io::Read + Send + 'static>, into: &Arc<Mutex<Vec<String>>>) {
    let Some(stream) = stream else { return };
    let into = Arc::clone(into);
    std::thread::spawn(move || {
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            if let Ok(mut buffer) = into.lock() {
                buffer.push(kooch_core::strip_ansi(&line));
            }
        }
    });
}

#[cfg(test)]
mod compile_tests;
