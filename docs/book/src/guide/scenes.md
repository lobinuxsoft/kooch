# Scenes and Prefabs

A **scene** is a file of entities: `.scene`, plain RON, readable in a diff. A **prefab** is the
same format with one root — an entity and its children, saved to be placed many times. Both live
under `assets/`, and both are addressed by a `Guid` from their `.meta`, so moving or renaming the
file breaks nothing that points at it.

## Scenes

### More than one open

The editor can hold several scenes at once. Right-click a scene in the Asset Browser: **Open
Scene** replaces what is open, **Open Additive** loads it beside the others. The World panel lists each scene
as its own group, and the active one is where new entities go.

Each open scene saves to its own file. An entity belongs to exactly one.

### Which one the game starts with

Right-click a scene in the Asset Browser → **Set as Main Scene**. That writes `main_scene` into
`project.kooch`, and both Play and a shipped build open it first. The full order the engine tries
is in [Getting Started](getting-started.md#loading-a-scene).

### Changing scene from the game

Not yet. Today only the engine's own boot and the editor load a scene; a game has no call to go
from the menu to a level. That is [#1920](https://github.com/lobinuxsoft/kooch/issues/1920).

Until it lands, a level can be a prefab spawned into one scene — see below.

## Prefabs

### Making one

Two ways, same result:

- Right-click an entity in the World panel → **Save as Prefab**. It writes the entity and its
  children to `assets/`.
- Drag the entity from the World panel onto a folder in the Asset Browser. It is written there.

One entity at a time. A prefab is a tree with one root, so a multi-selection is either several
prefabs or something that is not a tree.

### Placing one

Drag the prefab from the Asset Browser into the viewport or the World panel, or right-click it →
**Instantiate into Scene**. The copy you get is an **instance**: it remembers which prefab it came from.

### Overrides

Change a field on an instance and that field becomes an **override**: it keeps your value when the
prefab changes. Everything else keeps following the prefab.

In the Inspector, a component the instance has overridden gets a **Revert ⟨component⟩ to Prefab**
button.
Right-click the instance in the World panel → **Revert to Prefab** drops every override at once.

Reverting is what makes an override safe. Without it, an accidental gizmo drag would detach that
transform from the prefab for good.

### Editing the prefab itself

Select the prefab in the Asset Browser: the Inspector shows its entities and their components, and
edits there change the prefab, not any instance. **Save** writes the file and pushes the new values
out to every instance in the open scenes — except the fields each instance overrode.

## Spawning from code

At runtime a prefab is a **blueprint**: spawning it builds the entities, and the result has no link
back. Editing the file later does not reach anything already spawned. That is what a bullet, a
pickup or an enemy wave wants.

The engine ships no spawner component, because *when* and *where* is every game's own decision.
What it gives you is the field type and the call. A field typed like this gets a prefab picker in
the Inspector:

```rust
# use kooch::prelude::*;
#[derive(Default, Reflect)]
pub struct Spawner {
    #[reflect(asset = "kooch_ecs::scene::document::SceneDocument")]
    pub prefab: Option<Guid>,
    pub every: f32,
    pub wait: f32,
}
impl Component for Spawner {}

/// Spawns each spawner's prefab at the spawner, every `every` seconds.
#[system]
pub fn spawn_waves(resources: &mut Resources) {
    let dt = Time::of(resources).delta_secs();
    let mut due = Vec::new();
    Query::<(&mut Spawner, &Transform)>::new(resources).for_each(|(spawner, at)| {
        spawner.wait -= dt;
        if !(spawner.wait > 0.0) {
            spawner.wait = spawner.every;
            if let Some(prefab) = spawner.prefab {
                due.push((prefab, at.position));
            }
        }
    });
    for (prefab, position) in due {
        match kooch::kooch_ecs::scene::prefab::spawn(prefab, resources) {
            Ok(root) => {
                if let Some(transform) = Query::<&mut Transform>::new(resources).get(root) {
                    transform.position = position;
                }
            }
            Err(error) => warn_once!("spawner: {error}"),
        }
    }
}
```

`spawn` returns the root; `spawn_members` returns the root and every entity it built, in the
prefab's order, for when you need to reach a child. The entities join the active scene.

Spawn in a system that is allowed to change the world — `spawn` needs `&mut Resources`, so collect
what to spawn first and spawn after the query, as above.

## What a scene does not keep

Entities spawned during Play are not saved: Stop puts every open scene back the way it was
authored, the spawned bullets included. A scene file only ever holds what you placed in the editor.
