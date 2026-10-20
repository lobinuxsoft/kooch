# Where component values live

A query over two components costs **2.9 ns per entity**, and that number does not change between a
thousand entities and a million. It used to be 18.8 ns at a thousand and 66.5 ns at a million,
because every component of every entity was one `HashMap` lookup into memory the prefetcher never
saw coming.

## The shape

```
Entity ──▶ Archetype { entities: [Entity], rows: [Option<TableRow>], table: TableId }
                              ▲                      ▲
                              └── parallel lists ────┘
                                  the walk reads both from one index

Table { columns: [Column], ids: [StorageId], entities: [Entity] }

  Column ── a dense, type-erased array: every value of one component, end to end
```

An archetype groups the entities that have the same set of components. A table holds one column per
component of that set, and one row per entity. The archetype hands out the **row**, not a key — so
walking it is pointer arithmetic, and two entities next to each other in the walk are next to each
other in memory.

Two archetypes with the same component set share one table. A table is a property of the set.

## The inbox

Every site in the engine writes a component value and *then* changes the entity's archetype:

```rust
registry.get_cpu_mut::<Parent>().unwrap().insert(child, Parent { entity: parent });
move_entity::gained(resources, child, TypeId::of::<Parent>());
```

Between those two lines the entity's table has no column for `Parent`, so the value lands in the
per-type map instead. That map is the **inbox**; the column is the home. The archetype change
promotes the value from one to the other, and the promotion drains the inbox — a component's value
is never in two places.

So `get_cpu` returns a **view** that looks in the column first and the inbox second. That is a
lookup, not a preference: whichever holds it is where it is.

## What a plugin author has to know

| | |
|---|---|
| `registry.get_cpu::<T>()` | `Option<ComponentView<T>>`, not `Option<&ComponentStorage<T>>`. `get`, `contains`, `len`, `iter`, `revision` all work as before. It is `Copy`. |
| `registry.get_cpu_mut::<T>()` | `Option<ComponentViewMut<T>>`. Bind it `mut`. `into_mut(entity)` when the view is a temporary and the `&mut T` has to outlive it. |
| **`move_entity::take::<T>(resources, entity)`** | The only complete way to remove a component. |
| `view.unpark(entity)` | Takes a value out of the **inbox only**. Named for what it can do: a column loses a row only when the whole table does. Fine when an archetype change follows; it removes nothing on its own. |
| **`move_entity::move_in` / `gained` / `lost`** | The only way to change an archetype. There is no longer a `register_entity`. |

## Why the funnel is the only door

The row is recorded twice: on the archetype, for the walk, which must not hash; and in the component
registry, for a reader that has no archetype in hand — the inspector, scene save, undo. The two are
the same fact, and `move_entity` is what keeps them equal.

🔴 Change an archetype without moving the row and the entity reads **another entity's components**.
Nothing errors; a query and `get_cpu` simply answer differently about the same entity.
`move_entity::rows_agree` is the check, and a test asserts it over a world built the real way.

## Deliberately not here

- **A sparse-set storage kind.** It buys cheaper insert and remove by not moving rows, and costs a
  second code path through every query, more `unsafe`, and an `Option<TableRow>` in types that would
  not need one. Bevy's own maintainers [proposed removing
  theirs](https://github.com/bevyengine/bevy/discussions/19164) for exactly those reasons. An
  `Option<T>` in a table column covers the churning-component case without any of it.
- **Change detection.** Bevy's columns carry added and changed ticks. This engine has none, and
  porting the column shape without them is simpler and correct. Revisions are per component type,
  on the registry slot, which is what the editor's frame cache needs (#1561).
- **Chunked tables.** Unity splits an archetype into 16–64 KiB chunks, which matters for handing
  slices to parallel jobs. Whole-table columns iterate just as densely; the chunk is for the
  scheduler we do not have yet.
