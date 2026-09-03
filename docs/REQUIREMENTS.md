# Requirements — what a machine needs, and how that is known

A project made with this editor **compiles the engine**: the gameplay is
native Rust and links it as an `rlib`. So the machine that opens a
project needs a Rust toolchain and the system libraries the engine's
`*-sys` crates link against.

The editor checks this once at startup and, when something is missing,
shows the requirement, the reason, and the exact command for *this*
machine — and can run it. See `crates/kooch_editor_core/src/preflight.rs`
and `install.rs`.

## 🔴 How this list is decided

**By probing what the build probes, never by reading a table.**
`pkg-config --exists alsa` is literally what `alsa-sys`'s build script
runs, so its answer is the build's answer rather than a guess about it.

That rule was learned the expensive way. A machine that could not build
was diagnosed from a documentation table as missing four `-devel`
packages; the actual build failed on exactly **one**, and the other three
are `dlopen`ed by winit at runtime and were never needed.

So a package belongs on this list only if a `*-sys` crate in the
dependency tree links it **at build time**, and the entry names the crate
that does.

## Required — the build fails without these

| Requirement | Who needs it | Probe | Fedora / Atomic | Debian | Arch | Windows |
|---|---|---|---|---|---|---|
| **Rust** | everything; a project compiles the engine | `cargo --version` | rustup | rustup | rustup | rustup |
| **ALSA headers** | `alsa-sys` ← `cpal` ← `kooch_audio` | `pkg-config --exists alsa` | `alsa-lib-devel` | `libasound2-dev` | `alsa-lib` | — |
| **udev headers** | `libudev-sys` ← `gilrs-core` — how a gamepad is seen | `pkg-config --exists libudev` | `systemd-devel` | `libudev-dev` | `systemd-libs` | — |
| **A C compiler** | `metis-sys` and `zstd-sys` build C sources | `cc --version` | `gcc` | `build-essential` | `base-devel` | MSVC |
| **Vulkan headers** | `dlss_wgpu`'s build script | the header file itself | `vulkan-headers` | `libvulkan-dev` | `vulkan-headers` | Vulkan SDK |

**The Vulkan probe is a file test, not `pkg-config --exists vulkan`.**
That query answers for the *loader*, which is on every machine that runs
a game, and says nothing about whether a compiler could find
`vulkan/vulkan.h`. Asking the wrong question is how this requirement was
missed the first time.

**The C compiler is probed on Linux only.** On Windows the compiler is
MSVC, found through `vswhere` rather than on `PATH`, and answering that
wrongly would refuse builds that work.

**Rust is never a distribution package**, even where one exists. A
toolchain installed that way cannot be updated by `rustup update` and
cannot add a target with `rustup target add` — which is how a
cross-compiled build stops being possible.

## Not required — offered in the same step

| | Why | All Linux |
|---|---|---|
| **mold** | halves the link, but only beside split debug info — see below | `mold` |

### Measured, because the guess was wrong

One-line change to a project, warm cache, rebuilding its authoring
binary:

| | |
|---|---|
| as it was | **14.6 s** |
| `mold` alone | 12.5 s |
| split debug info alone | 11.7 s |
| **both** | **5.9 s** |

🔴 **Neither is worth much alone** — 14 % and 20 % — **and together they
are 2.5x.** They interact: a fast linker is not fast while it is still
copying 600 MB of DWARF into the output, and not copying it does not help
while the linker itself is the slow part. The binary drops from 635 MB
to 302 MB, which is the same fact seen from the other side.

The editor applies both to every project build it drives:
`CARGO_PROFILE_DEV_SPLIT_DEBUGINFO=unpacked` always, since it needs
nothing installed, and `-C link-arg=-fuse-ld=mold` when mold is there.

⚠️ **The first build after they change is a full one** — both are part
of cargo's fingerprint, so every dependency rebuilds once. Measured at
85-92 s here. Every build after it is the fast one.

They are set on the command rather than in a committed
`.cargo/config.toml`, because that file would force `mold` on every
machine and mold is optional — a machine without it would stop building
entirely.

Listed for the reason the whole check exists: on an image-based system,
a package found out about later costs **another reboot**. One command,
pasted once.

Kept in its own section on purpose. A list that mixes "you cannot build
without this" with "this would be faster" is one people read diagonally,
and reading diagonally is how the three unnecessary packages above got
installed.

## What is deliberately absent

- **wayland, libxkbcommon, libX11.** `winit` `dlopen`s them at runtime.
  A machine running a desktop already has them, and a `-devel` package
  is not what they need.
- **Vulkan loader.** Needed to *run*, not to build, and present wherever
  a game runs at all.
- **mingw / MSVC cross toolchains.** Only a Windows cross-build needs
  `mingw64-gcc-c++`, and that surfaces from the build panel with the
  target that asked for it.

## Installing

The editor builds the command for the package manager it detects —
`rpm-ostree`, `dnf`, `apt`, `pacman` or `winget` — reading `ID` **and**
`ID_LIKE` from `/etc/os-release`, because this project's own
distribution reports `ID=yaguarete` with `ID_LIKE="bazzite fedora"`.

The **Install** button runs it through `pkexec`, so authentication is
the desktop's own prompt rather than one of ours.

🔴 **On an image-based distribution it then restarts the machine**,
because the new image is not the running one until it does. Three things
guard that:

1. **It refuses while any scene is dirty.** A restart with unsaved work
   is the editor destroying the author's work to save them a paste.
2. **A failed install restarts nothing.** On an atomic system the
   routine failure is "that package does not exist", which a reboot
   would hide.
3. **It never installs Rust.** `rustup` installs into the invoking
   user's home; through a privileged helper it would land in root's, and
   the user's shell would still find nothing.

## Adding to this list

1. Find the `*-sys` crate: `cargo tree -i <crate>-sys -e normal`.
2. Confirm it links at **build** time, not `dlopen`s at runtime.
3. Add the probe that crate's build script actually runs.
4. Add the package name per installer, and a row here naming the crate.

A requirement whose row cannot name the crate that needs it does not
belong on the list.
