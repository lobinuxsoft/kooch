//! Describing a component type a plugin owns.
//!
//! A plugin's component types do not exist in the engine's binary, so
//! the engine cannot name them. What it receives instead is a
//! *description*: a type name and its fields. That is what the editor's
//! Inspector draws and what the scene format writes, and it is the same
//! shape `DynamicComponents` already stores components under.
//!
//! These are ordinary Rust types. The plugin is a `dylib` built by the
//! same compiler as the engine, so `String` and `Vec` cross the boundary
//! as themselves — there is no reason to hand-roll pointer-and-length
//! pairs, and every reason not to.

/// The type of a component field.
///
/// Mirrors `kooch_ecs::reflect::FieldKind`. The two are separate because
/// `kooch_ecs` pulls in wgpu, glam, serde and ron — a plugin should not
/// link a GPU stack to say that a field holds an `f32`. The engine maps
/// between them in one place, and a parity test keeps them in step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldKind {
    /// 32-bit float.
    F32,
    /// 64-bit float.
    F64,
    /// Unsigned 8-bit integer.
    U8,
    /// Unsigned 16-bit integer.
    U16,
    /// Unsigned 32-bit integer.
    U32,
    /// Unsigned 64-bit integer.
    U64,
    /// Signed 8-bit integer.
    I8,
    /// Signed 16-bit integer.
    I16,
    /// Signed 32-bit integer.
    I32,
    /// Signed 64-bit integer.
    I64,
    /// Boolean.
    Bool,
    /// UTF-8 string.
    String,
    /// Two-component vector.
    Vec2,
    /// Three-component vector.
    Vec3,
    /// Four-component vector.
    Vec4,
    /// Quaternion.
    Quat,
    /// 4x4 matrix.
    Mat4,
    /// Asset reference, addressed by GUID.
    AssetRef,
    /// Reference to another entity.
    EntityRef,
    /// Nested reflected struct.
    Nested,
}

impl FieldKind {
    /// Every kind, in declaration order.
    ///
    /// Exists so the engine's parity test can assert it knows how to map
    /// all of them — a new kind added here without a mapping fails the
    /// build rather than silently drawing the wrong widget.
    pub const ALL: &'static [FieldKind] = &[
        FieldKind::F32,
        FieldKind::F64,
        FieldKind::U8,
        FieldKind::U16,
        FieldKind::U32,
        FieldKind::U64,
        FieldKind::I8,
        FieldKind::I16,
        FieldKind::I32,
        FieldKind::I64,
        FieldKind::Bool,
        FieldKind::String,
        FieldKind::Vec2,
        FieldKind::Vec3,
        FieldKind::Vec4,
        FieldKind::Quat,
        FieldKind::Mat4,
        FieldKind::AssetRef,
        FieldKind::EntityRef,
        FieldKind::Nested,
    ];
}

/// One field of a plugin-declared component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldSchema {
    /// Field name as it appears in the Inspector and the scene file.
    pub name: String,
    /// What the field holds.
    pub kind: FieldKind,
    /// The field's doc comment, shown as an Inspector tooltip (#737).
    /// Empty when the field has none.
    ///
    /// Travels the boundary because a project's own components are
    /// exactly the ones whose meaning the engine cannot guess. A
    /// `GroundMovement.acceleration` is in units only its author knows.
    pub doc: String,
}

impl FieldSchema {
    /// Describes a field.
    pub fn new(name: impl Into<String>, kind: FieldKind) -> Self {
        Self {
            name: name.into(),
            kind,
            doc: String::new(),
        }
    }

    /// Attaches the field's doc comment, shown as an Inspector tooltip.
    pub fn with_doc(mut self, doc: impl Into<String>) -> Self {
        self.doc = doc.into();
        self
    }
}

/// A component type a plugin declares to the engine.
///
/// The type name is the identity — the engine keys stored components by
/// it — so it must be stable across rebuilds of the plugin. Use a fully
/// qualified path such as `"my_game::Health"`, not `"Health"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentSchema {
    /// Fully qualified type name.
    pub type_name: String,
    /// The component's fields. Empty is legal — a marker component.
    pub fields: Vec<FieldSchema>,
    /// The values a fresh one holds, as the RON the engine writes into a
    /// scene: `[("acceleration", F32(20.0)), ...]`. Empty for a marker.
    ///
    /// # Why a string and not typed values
    ///
    /// A field's *value* is a nineteen-variant enum carrying vectors,
    /// quaternions, matrices and asset guids. Mirroring that across the
    /// plugin boundary — the way [`FieldKind`] mirrors its own — would be
    /// nineteen more things to keep in parity for no gain, since the
    /// engine already has one serialised form for exactly these values
    /// and writes it to every scene file.
    ///
    /// # Why it has to travel at all
    ///
    /// Only the plugin knows the type's `Default`. Without it the editor
    /// knows a component has two `f32` and not that they are 20 and 8, so
    /// adding it to a prefab either fails or silently produces zeroes —
    /// and a body that accelerates at 0 toward a top speed of 0 reads as
    /// a broken component, not as a missing default.
    pub defaults: String,
}

impl ComponentSchema {
    /// Describes a component with no fields — a marker.
    pub fn new(type_name: impl Into<String>) -> Self {
        Self {
            type_name: type_name.into(),
            fields: Vec::new(),
            defaults: String::new(),
        }
    }

    /// Adds a field, for building a schema fluently.
    #[must_use]
    pub fn with_field(mut self, name: impl Into<String>, kind: FieldKind) -> Self {
        self.fields.push(FieldSchema::new(name, kind));
        self
    }
}

/// Why the engine refused a [`ComponentSchema`].
///
/// Distinguishable because the fixes differ: an empty name is the
/// plugin author's mistake, a taken name is a collision with another
/// plugin, and a missing bridge means the host built an `App` with no
/// ECS at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterError {
    /// The type name was empty.
    EmptyName,
    /// A field name was empty.
    EmptyFieldName {
        /// Index of the offending field.
        index: usize,
    },
    /// Another type already holds this name.
    NameTaken {
        /// The name that was already claimed.
        type_name: String,
    },
    /// The host has no component registry wired.
    NoRegistry,
}

impl std::fmt::Display for RegisterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyName => write!(f, "component type name is empty"),
            Self::EmptyFieldName { index } => write!(f, "field {index} has an empty name"),
            Self::NameTaken { type_name } => {
                write!(f, "a component named {type_name} is already registered")
            }
            Self::NoRegistry => write!(f, "the host has no component registry"),
        }
    }
}

impl std::error::Error for RegisterError {}

#[cfg(test)]
mod tests;
