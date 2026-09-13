//! Describing a component type a plugin owns: the engine cannot name it, so it receives a type name
//! and fields — what the Inspector draws and scenes store. Plain Rust types, since both sides share
//! a compiler.

/// The type of a component field, mirroring `kooch_ecs::reflect::FieldKind` so a plugin does not
/// link a GPU stack; a parity test keeps them in step.
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
    /// Every kind, in declaration order, so the engine's parity test fails the build on an unmapped
    /// kind.
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
    /// The field's doc comment, shown as an Inspector tooltip (#737); empty when there is none. A
    /// project's own units are only known to its author.
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

/// A component type a plugin declares. The type name is its stored identity, so keep it stable and
/// fully qualified, like `my_game::Health`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentSchema {
    /// Fully qualified type name.
    pub type_name: String,
    /// The component's fields. Empty is legal — a marker component.
    pub fields: Vec<FieldSchema>,
    /// The values a fresh one holds, as the RON the engine writes to scenes; empty for a marker.
    /// A string because the engine already has that serialised form; it has to travel because only
    /// the plugin knows the type's `Default`.
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

/// Why the engine refused a [`ComponentSchema`] — distinct variants because the fixes differ.
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
