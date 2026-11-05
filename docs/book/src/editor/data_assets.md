# Data assets

A data asset is a file of your own type: tuning, a level's rules, an enemy's stats. You author it in
the editor and a system reads it, like a `ScriptableObject` in Unity. Any type with `Reflect` and
`Default` can be one:

```rust
#[derive(Reflect)]
pub struct BallConfig {
    /// N·m at a full pull.
    pub max_torque: f32,
    pub max_spin: f32,
}

impl Default for BallConfig {
    fn default() -> Self {
        Self { max_torque: 6.0, max_spin: 40.0 }
    }
}

impl DataAsset for BallConfig {
    const EXTENSION: &'static str = "ballconfig";
}
```

The `impl` line has to start like that, in a file under `src/` outside `src/tools/`: the editor
finds it by reading it. After the next build, the folder menu in the Asset Browser offers
**New BallConfig**. A new file holds the type's defaults, and the Inspector edits its fields with
undo. Saving reaches a running game at once.

## Reading one

A component points at the file with an asset field, and a system loads it by guid:

```rust
#[reflect(asset = "my_game::registrations::components::ball::BallConfig")]
pub config: Option<Guid>,
```

```rust
if let Some(config) = data_asset::<BallConfig>(resources, guid) {
    // …
}
```

The name in `asset` is the type's full path, `registrations` included, because the project's modules
live inside that file.

## Changing the type

The file stores each field by name, so the type can move on:

- A new field takes its default.
- A removed field is dropped the next time the file is saved.
- A field whose type changed goes back to its default in the editor; the game logs a warning.

A field's doc comment shows no tooltip in the Inspector yet, and no `#[reflect(...)]` range or
choices apply, because those stay in the project's library.

## From an editor action

An [action](./panels.md#running-your-code) has the type, so it can write one:

```rust
use kooch::kooch_ecs::reflect::{data_fields, data_text};

let config = BallConfig { max_torque: 12.0, max_spin: 60.0 };
call.write_asset("config/fast.ballconfig", data_text(&data_fields(&config)));
```
