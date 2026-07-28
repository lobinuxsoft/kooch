# Writing a Component

<!-- toc -->

A component is a plain struct that derives `Reflect` and implements `Component`. That is the
whole contract — the Inspector, scene serialisation and the Add Component menu all follow from
the derive.

## The smallest one that works

```rust
use oh_my_engine::ome_ecs::Reflect;
use oh_my_engine::ome_ecs::component::Component;

/// How much damage this entity can still take.
#[derive(Default, Reflect)]
pub struct Health {
    pub current: f32,
    pub max: f32,
}

impl Component for Health {}
```

Create it from the editor (which drops this scaffold in `src/` and regenerates
`registrations.rs`), or write the file yourself and press Register Scripts.

**Public fields show up in the Inspector automatically.** No attribute is required to opt in;
attributes exist to opt *out*, or to say something the type alone cannot.

## What the Inspector can draw

Each field's Rust type maps to a `FieldKind`, and the kind decides the widget:

| Rust type | Widget |
|---|---|
| `f32`, `f64` | Drag value |
| `u8`…`u64`, `i8`…`i64` | Drag value, clamped to the type |
| `bool` | Checkbox |
| `String` | Text field |
| `Vec2`, `Vec3`, `Vec4` | Component-wise drag values |
| `Quat` | Euler angles, in degrees |
| `Mat4` | Decomposed to translation / rotation / lossy scale, read-only |
| `Option<Guid>` + `#[reflect(asset = "…")]` | Typed asset picker |
| `Entity` | Entity reference — **read-only today**, see [#655](https://github.com/lobinuxsoft/oh_my_engine/issues/655) |
| A struct that also derives `Reflect` | Nested, drawn inline |

> **`glam` is not re-exported yet.** `Vec3`, `Quat` and `Mat4` are `glam` types, and a project
> reaches them only by adding `glam = "0.33"` to its own `Cargo.toml` — the same version the
> engine pins, or the types will not match. Tracked in
> [#657](https://github.com/lobinuxsoft/oh_my_engine/issues/657).

Anything outside that list — `Vec<T>`, `HashMap<K, V>`, your own enums — is **not supported
yet**. Recursive reflection for nested types and collections is
[#649](https://github.com/lobinuxsoft/oh_my_engine/issues/649). Until it lands, a field of an
unsupported type needs `#[reflect(skip)]` or the derive will not compile.

## Attributes

### On the struct

```rust
#[derive(Default, Reflect)]
#[reflect(category = "Gameplay")]      // groups it in the Add Component menu
#[reflect(inspector = "read_only")]    // "hidden" | "read_only" | "editable" (default)
pub struct Health { … }
```

### On a field

```rust
#[derive(Default, Reflect)]
pub struct Weapon {
    /// Not shown, not serialised. Use for runtime caches and for types
    /// the Inspector has no representation for.
    #[reflect(skip)]
    cached_target: Option<Entity>,

    /// A typed asset picker instead of a raw Guid text field.
    #[reflect(asset = "Mesh")]
    pub projectile: Option<Guid>,

    /// A dropdown of named values instead of a bare integer.
    #[reflect(choices = FIRE_MODE_CHOICES)]
    pub fire_mode: u32,

    /// A row of checkboxes instead of a bitmask you compute in your head.
    #[reflect(bits = DAMAGE_TYPE_BITS)]
    pub damage_types: u32,

    /// Only drawn when another field says it is relevant.
    #[reflect(shown_when = BURST_ONLY)]
    pub burst_count: u32,
}
```

`choices`, `bits` and `shown_when` take a path to a constant, not a string literal — so the
same table is used by the Inspector and by your code, and they cannot drift apart.

`shown_when` is what keeps a component with many mutually-exclusive fields readable: the
engine's own `Joint` uses it so a hinge does not show you spring stiffness.

## Registration

You do not write it. The editor scans `src/`, finds `impl Component for Health`, and
regenerates `registrations.rs` with both halves:

```rust
// Registers the type with the running ECS — scene save/load and the Inspector.
registry.register_cpu_reflected::<Health>();

// Describes the type to a standalone editor that loaded this dylib.
declare_component::<Health>(engine);
```

The component's name comes from `std::any::type_name::<T>()`, so there is exactly one name
for a type and no way for two sides to disagree about it.

## What survives a save

A component is saved as its reflected fields. Two consequences worth knowing before you design
a component:

- **`#[reflect(skip)]` fields are not saved.** They are reconstructed by your code, or they
  are gone.
- **A reference to another entity has to be persistent, not live.** An `Entity` is an index
  and a generation, which mean nothing after a reload. `EntityRef::Persistent` is the type
  that survives, and resolving one to the other is the save path's job. Getting this wrong is
  how [#655](https://github.com/lobinuxsoft/oh_my_engine/issues/655) happened — the engine's
  own `Joint` still holds raw `Entity` fields and cannot be saved because of it.
