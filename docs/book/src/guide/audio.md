# Audio

🔴 **Audio is a backend today, not a feature.** `kooch_audio` decodes and plays sounds through
kira, but nothing in the engine calls it: there is no `AudioSource` component, no audio asset in
the Asset Browser, no listener, and the editor plays nothing. All of that is
[#1645](https://github.com/lobinuxsoft/kooch/issues/1645).

What works now is a game creating the backend itself and playing sounds from code. This page is
that, so a game is not silent while #1645 is open. Audio needs the `audio` feature.

## What the backend does

| Call | Does |
|---|---|
| `load_sound(&bytes)` | Decodes an `.ogg`, `.wav`, `.flac` or `.mp3` into memory, returns a `SoundHandle` |
| `play(sound, PlayParams)` | Starts an instance, returns an `InstanceHandle`. One sound plays any number of times at once |
| `stop(instance)` | Stops one instance. A stale handle is a no-op |
| `set_instance_volume`, `set_master_volume` | Linear gain: 1 unchanged, 0 silent |
| `unload_sound(sound)` | Frees the decoded data. What is playing finishes |

`PlayParams` is `volume`, `pitch` (a rate: 2 is an octave up) and `looping`.

Not there yet, all in #1645: positional sound (`set_listener` stores a pose and nothing reads it),
streaming for long music, buses, a voice limit, pause and resume.

## Playing a sound from a game

Create the backend once, keep it as a resource, and read the files through the `AssetServer`, with
a path relative to `assets/`.

```rust,no_run
# use kooch::prelude::*;
# use kooch::kooch_audio::KiraBackend;
/// The game's sounds, decoded once.
pub struct Sounds {
    audio: Box<dyn AudioBackend>,
    jump: Option<SoundHandle>,
}

/// Opens the audio device and decodes the sounds. A machine with no device plays nothing.
pub fn open_audio(resources: &mut Resources) {
    let Ok(backend) = KiraBackend::new() else {
        warn!("no audio device: the game runs silent");
        return;
    };
    let mut audio: Box<dyn AudioBackend> = Box::new(backend);
    let jump = resources
        .get_mut::<AssetServer>()
        .and_then(|server| server.read_bytes("sounds/jump.ogg").ok())
        .and_then(|bytes| audio.load_sound(&bytes).ok());
    resources.insert(Sounds { audio, jump });
}

/// Plays the jump sound, a little higher each time so repeats do not sound mechanical.
pub fn play_jump(resources: &mut Resources, pitch: f32) {
    let Some(sounds) = resources.get_mut::<Sounds>() else {
        return;
    };
    if let Some(jump) = sounds.jump {
        let params = PlayParams { pitch, ..PlayParams::default() };
        let _ = sounds.audio.play(jump, params);
    }
}
```

Run `open_audio` once — at `Stage::Startup`, from the app's `main` — and call `play_jump` from
whatever system decides a jump happened.

🔴 **This works in the editor and with `cargo run`, not in an exported build.** The packer carries
only the files some engine loader claims, and no loader claims audio yet, so `.ogg` and `.wav`
files are left out of `assets.kpack` and `read_bytes` finds nothing. The audio asset of #1645 is
what fixes it. Until then, a game that must ship sound reads its files from a folder beside the
executable with `std::fs::read`.

## What the engine does not do for you yet

- **The engine stops no sound.** A loop plays until your code calls `stop`, so keep the
  `InstanceHandle` of anything that loops, and stop it when the game pauses or leaves the level.
- **Nothing is freed for you.** A scene change does not unload sounds; `unload_sound` does.
- **No sound is positional.** Volume by distance, if a game needs it before #1645, is a gain the
  game computes and sets with `set_instance_volume`.
