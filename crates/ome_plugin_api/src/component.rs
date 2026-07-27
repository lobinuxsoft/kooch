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
/// Mirrors `ome_ecs::reflect::FieldKind`. The two are separate because
/// `ome_ecs` pulls in wgpu, glam, serde and ron — a plugin should not
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
}

impl FieldSchema {
    /// Describes a field.
    pub fn new(name: impl Into<String>, kind: FieldKind) -> Self {
        Self {
            name: name.into(),
            kind,
        }
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
}

impl ComponentSchema {
    /// Describes a component with no fields — a marker.
    pub fn new(type_name: impl Into<String>) -> Self {
        Self {
            type_name: type_name.into(),
            fields: Vec::new(),
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
mod tests {
    use super::*;

    #[test]
    fn a_schema_is_built_fluently() {
        let schema = ComponentSchema::new("my_game::Health")
            .with_field("current", FieldKind::U32)
            .with_field("regen", FieldKind::F32);

        assert_eq!(schema.type_name, "my_game::Health");
        assert_eq!(schema.fields.len(), 2);
        assert_eq!(schema.fields[1].kind, FieldKind::F32);
    }

    /// A marker component is a real thing, not a half-built schema.
    #[test]
    fn a_marker_has_no_fields() {
        assert!(ComponentSchema::new("my_game::Player").fields.is_empty());
    }

    /// `ALL` exists so the engine can prove it maps every kind. If a
    /// variant is added without listing it here, the mapping would
    /// silently skip it.
    #[test]
    fn all_lists_every_kind() {
        // Adding a variant without extending ALL leaves this stale, and
        // the engine-side parity test then fails loudly.
        assert_eq!(FieldKind::ALL.len(), 20);
        assert_eq!(FieldKind::ALL[0], FieldKind::F32);
        assert_eq!(FieldKind::ALL[19], FieldKind::Nested);

        let mut seen = std::collections::HashSet::new();
        for kind in FieldKind::ALL {
            assert!(seen.insert(*kind), "{kind:?} listed twice in ALL");
        }
    }
}
