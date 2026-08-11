# Shipping a Game

A build turns a project into a folder someone else can run: an
executable, and the assets it needs, and nothing else. This page is what
the Build panel does and how to make it produce something that starts on
a machine that is not yours.

## Presets

A `.buildpreset` is one way of building the project — "Linux release",
"Windows", "handheld". It is an ordinary asset: it lives in `assets/`,
it is created from the Asset Browser's context menu, and it is edited in
the **Inspector** like anything else. The Build panel only holds the
list, the button, and cargo's output.

Presets belong in version control. They are configuration, and a project
usually has more than one.

## What a build produces

```text
build/
  My Game.x86_64      the executable, named for its target
  assets.kpack        the scenes and everything they reference
```

The extension follows the target — `.exe` on Windows, the architecture
on Linux, the same convention Unity and Godot use. A folder holding both
platforms is then unambiguous.

## What travels, and what does not

Only what the game reaches. Packaging walks the scenes and prefabs,
collects the GUIDs they reference, and takes those files and their
`.meta` sidecars. An asset nobody uses stays behind, and so does every
`.buildpreset` — a build does not ship the instructions for making
itself.

Source does not travel either. A shipped game is a compiled binary; the
`src/` folder is what produced it, not part of it.

This is why anything imported through the Asset Browser lands under
`assets/` whatever folder was selected. The split is what makes "what
does the game need" answerable at all:

| Folder    | Holds                                | Ships |
| --------- | ------------------------------------ | ----- |
| `assets/` | scenes, meshes, textures, materials  | the parts that are used |
| `src/`    | components, systems, `main.rs`       | compiled in, not copied |
| `.kooch/` | the pack key, local state            | never |

## The asset pack

With `pack_assets` on — the default — everything lands in a single
encrypted `assets.kpack` rather than as loose files. It is compressed
with zstd and sealed with AES-256-GCM, including the index, so the file
does not even reveal the *names* of what is inside it.

Scenes go in the pack too. A scene is the structure of the whole game;
leaving it in plain text beside an encrypted pack would protect the
textures and publish the design.

Turn `pack_assets` off while working out why a build behaves differently
from the editor — then the files are right there to look at.

> ⚠️ The key has to be inside the binary for the binary to read the pack.
> This raises the cost of taking your assets; it does not make it
> impossible, and nothing does. A game hands its meshes to the GPU in the
> clear because that is what drawing them means.

### The key

Each project gets its own, generated once and kept at
`.kooch/pack.key`. It is not in version control — a repository carrying
it has published it, and history keeps it published after the file is
deleted. That is the same line Godot draws between `export_presets.cfg`
and its encryption key.

One key per project, so breaking one says nothing about the next.

For CI, set `KOOCH_PACK_KEY` to the key's hex and nothing is written into
the checkout. Keep it in the secret store, not the repository.

## Measuring the game where it runs: `profiling`

Tick **`profiling`** and the build carries the profiler: it opens
`0.0.0.0:8585` and streams every frame to the editor's Profiler panel,
which is the only way to find out where a frame goes on the hardware the
game has to run on. See [Profiling](../architecture/profiling.md).

🔴 **Never on a build anyone else receives.** It is a listening socket
and a background thread. Off is not "switched off": with the feature
absent, every scope in the engine expands to nothing at compile time and
there is no socket to open.

Keep it as its own preset — "handheld, profiled" beside "handheld" — so
the ordinary build cannot acquire a socket because somebody forgot to
untick a box.

## Running on another machine: `min_glibc`

A game built on an up-to-date desktop often **refuses to start** on a
Steam Deck or a handheld, with a message about a missing symbol version.
glibc is forward compatible and not backward: a binary linked against
2.43 does not run against 2.42, and the error says nothing about what to
do.

Set **`min_glibc`** to the oldest version the build has to run on and it
links against that instead:

| Value  | Runs on                                    |
| ------ | ------------------------------------------ |
| empty  | this machine and anything newer — the default |
| `2.28` | Debian 10, RHEL 8, and everything since — what Godot's Linux exports target |
| `2.31` | Ubuntu 20.04 and newer |

Leave it empty while iterating locally; set it before handing the build
to anyone.

### What it needs

Two tools, neither of which requires root — which matters on an immutable
distribution, where there is no `dnf install` to reach for:

```sh
cargo install cargo-zigbuild
# and zig itself, one tarball from https://ziglang.org/download/
#   tar xf zig-*.tar.xz -C ~/.local/opt
#   ln -s ~/.local/opt/zig-*/zig ~/.local/bin/zig
```

The build checks for both **before** compiling and names what is missing.
A missing toolchain that surfaces ten minutes in, as a linker error, is
the thing that check exists to prevent.

The field is ignored for targets that are not `*-linux-gnu`; there is no
glibc to have a floor.

## Cancelling

The Build panel's Cancel stops cargo. Nothing is packaged, so a
half-built executable never reaches the output folder — packaging only
runs when cargo exits clean.

## Cross-compiling

Set `target_triple` to build for another platform. The target has to be
installed, and a preset naming one that is not fails immediately with the
`rustup target add` line to run rather than after a long compile.

Windows needs a mingw toolchain; the C23 workaround `metis` requires is
passed for you.
