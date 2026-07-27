//! Describing a component type across the FFI boundary.
//!
//! A plugin's component types do not exist in the engine's binary, so
//! they cannot cross as Rust types — there is no `TypeId` to agree on
//! and no layout the engine could name. What crosses instead is a
//! *description*: a type name and a list of fields, which is exactly
//! what the editor's Inspector draws and what the scene format writes.
//!
//! The engine already stores components it cannot name, keyed by type
//! name, in `DynamicComponents`. This is the other half of that: how a
//! plugin declares one in the first place.
//!
//! # Why constants rather than an enum
//!
//! [`field_kind`] mirrors `ome_ecs::reflect::FieldKind`, which is a
//! plain Rust enum with no `#[repr(u8)]` — its discriminants are not a
//! stable ABI and must not become one by accident. Repeating the values
//! here makes the wire format explicit and lets the engine's enum be
//! reordered without silently changing what a compiled plugin means.
//! The bridge maps between them once, in one place.

use std::ffi::c_void;

/// Field type discriminants, mirroring `ome_ecs::reflect::FieldKind`.
///
/// These values are a **wire format**: changing one invalidates every
/// plugin already compiled against it. Add new kinds at the end.
pub mod field_kind {
    /// 32-bit float.
    pub const F32: u8 = 0;
    /// 64-bit float.
    pub const F64: u8 = 1;
    /// Unsigned 8-bit integer.
    pub const U8: u8 = 2;
    /// Unsigned 16-bit integer.
    pub const U16: u8 = 3;
    /// Unsigned 32-bit integer.
    pub const U32: u8 = 4;
    /// Unsigned 64-bit integer.
    pub const U64: u8 = 5;
    /// Signed 8-bit integer.
    pub const I8: u8 = 6;
    /// Signed 16-bit integer.
    pub const I16: u8 = 7;
    /// Signed 32-bit integer.
    pub const I32: u8 = 8;
    /// Signed 64-bit integer.
    pub const I64: u8 = 9;
    /// Boolean.
    pub const BOOL: u8 = 10;
    /// UTF-8 string.
    pub const STRING: u8 = 11;
    /// Two-component vector.
    pub const VEC2: u8 = 12;
    /// Three-component vector.
    pub const VEC3: u8 = 13;
    /// Four-component vector.
    pub const VEC4: u8 = 14;
    /// Quaternion.
    pub const QUAT: u8 = 15;
    /// 4x4 matrix.
    pub const MAT4: u8 = 16;
    /// Asset reference, addressed by GUID.
    pub const ASSET_REF: u8 = 17;
    /// Reference to another entity.
    pub const ENTITY_REF: u8 = 18;
    /// Nested reflected struct.
    pub const NESTED: u8 = 19;

    /// Highest discriminant this build knows.
    ///
    /// The bridge rejects anything above it rather than guessing, so a
    /// plugin built against a newer API fails loudly on the field it
    /// cannot express instead of silently registering the wrong type.
    pub const MAX: u8 = NESTED;
}

/// One field of a component, as described by a plugin.
///
/// Borrowed, not owned: `name_ptr` must stay valid for the duration of
/// the `register_component` call, and the engine copies what it keeps.
/// A plugin can therefore build these from `&'static str` literals
/// without allocating.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FieldDesc {
    /// UTF-8 field name. Not NUL-terminated.
    pub name_ptr: *const u8,
    /// Length of the name in bytes.
    pub name_len: usize,
    /// One of the [`field_kind`] constants.
    pub kind: u8,
}

impl FieldDesc {
    /// Describes a field from a name and a [`field_kind`] constant.
    #[inline]
    pub const fn new(name: &str, kind: u8) -> Self {
        Self {
            name_ptr: name.as_ptr(),
            name_len: name.len(),
            kind,
        }
    }
}

/// A component type a plugin is declaring to the engine.
///
/// The name is the identity: the engine keys stored components by it,
/// so it must be stable across rebuilds of the plugin — a fully
/// qualified path such as `"my_game::Health"` rather than `"Health"`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ComponentDesc {
    /// UTF-8 fully qualified type name. Not NUL-terminated.
    pub name_ptr: *const u8,
    /// Length of the type name in bytes.
    pub name_len: usize,
    /// Pointer to `field_count` contiguous [`FieldDesc`].
    pub fields_ptr: *const FieldDesc,
    /// Number of fields. Zero is legal — a marker component.
    pub field_count: usize,
}

impl ComponentDesc {
    /// Describes a component from a name and a slice of fields.
    #[inline]
    pub const fn new(name: &str, fields: &[FieldDesc]) -> Self {
        Self {
            name_ptr: name.as_ptr(),
            name_len: name.len(),
            fields_ptr: fields.as_ptr(),
            field_count: fields.len(),
        }
    }
}

/// Outcome of a [`ComponentDesc`] registration.
///
/// Distinguishable failures, because they need different fixes: a bad
/// name is the plugin author's typo, an unknown field kind is a version
/// mismatch, and a missing bridge means the host never wired the ECS.
pub mod register_result {
    /// The component type was registered.
    pub const OK: u32 = 0;
    /// The type name or a field name was not valid UTF-8.
    pub const BAD_UTF8: u32 = 1;
    /// A field carried a kind this engine build does not know.
    pub const UNKNOWN_FIELD_KIND: u32 = 2;
    /// The engine has no component bridge registered — the host built
    /// an `App` without the ECS.
    pub const NO_BRIDGE: u32 = 3;
    /// A different type is already registered under this name.
    pub const NAME_TAKEN: u32 = 4;
}

/// Signature of the engine's component registration entry point.
///
/// Returns one of the [`register_result`] constants.
pub type RegisterComponentFn = extern "C" fn(ctx: *mut c_void, desc: *const ComponentDesc) -> u32;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_desc_borrows_its_name() {
        let f = FieldDesc::new("hp", field_kind::F32);
        assert_eq!(f.name_len, 2);
        assert_eq!(f.kind, field_kind::F32);
        let name = unsafe { std::slice::from_raw_parts(f.name_ptr, f.name_len) };
        assert_eq!(name, b"hp");
    }

    #[test]
    fn component_desc_points_at_its_fields() {
        const FIELDS: &[FieldDesc] = &[
            FieldDesc::new("current", field_kind::U32),
            FieldDesc::new("max", field_kind::U32),
        ];
        let d = ComponentDesc::new("my_game::Health", FIELDS);
        assert_eq!(d.field_count, 2);
        let fields = unsafe { std::slice::from_raw_parts(d.fields_ptr, d.field_count) };
        assert_eq!(fields[1].kind, field_kind::U32);
    }

    /// A marker component is legal and must not be confused with a
    /// failure to describe fields.
    #[test]
    fn zero_fields_is_valid() {
        let d = ComponentDesc::new("my_game::Player", &[]);
        assert_eq!(d.field_count, 0);
    }

    /// The kinds are a wire format. If this test is edited to match a
    /// change, every already-compiled plugin has been invalidated.
    #[test]
    fn field_kind_values_are_pinned() {
        assert_eq!(field_kind::F32, 0);
        assert_eq!(field_kind::BOOL, 10);
        assert_eq!(field_kind::VEC3, 13);
        assert_eq!(field_kind::NESTED, 19);
        assert_eq!(field_kind::MAX, field_kind::NESTED);
    }
}
