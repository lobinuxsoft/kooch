use std::fmt;

use ome_core::Guid;

use super::entity_ref::EntityRef;
use super::field::FieldKind;

mod non_finite;

/// Type-erased value for getting and setting reflected fields.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ReflectValue {
    /// A single-precision float, infinities and NaN included — see
    /// [`non_finite`] for why those need saying.
    #[serde(with = "non_finite::f32_repr")]
    F32(f32),
    #[serde(with = "non_finite::f64_repr")]
    F64(f64),
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    Bool(bool),
    String(String),
    Vec2(glam::Vec2),
    Vec3(glam::Vec3),
    Vec4(glam::Vec4),
    Quat(glam::Quat),
    Mat4(glam::Mat4),
    /// Typed asset reference. `guid` is `None` when the field is
    /// unassigned. `asset_type` is the static type name the field
    /// expects (e.g. `"ome_render::meshlet::MeshletMesh"`); the
    /// inspector uses it to filter `AssetDatabase::entries_of_type`
    /// when populating the picker.
    AssetRef {
        guid: Option<Guid>,
        asset_type: String,
    },
    /// Reference to another entity. `None` when the field points at
    /// nothing.
    ///
    /// A live component holds [`EntityRef::Live`]; a scene file holds
    /// [`EntityRef::Persistent`]. The save path converts one way and the
    /// load path's remapping pass converts back — see
    /// [`EntityRef`](super::EntityRef).
    EntityRef(Option<EntityRef>),
}

impl ReflectValue {
    /// Returns the [`FieldKind`] that matches this value variant.
    pub fn kind(&self) -> FieldKind {
        match self {
            Self::F32(_) => FieldKind::F32,
            Self::F64(_) => FieldKind::F64,
            Self::U8(_) => FieldKind::U8,
            Self::U16(_) => FieldKind::U16,
            Self::U32(_) => FieldKind::U32,
            Self::U64(_) => FieldKind::U64,
            Self::I8(_) => FieldKind::I8,
            Self::I16(_) => FieldKind::I16,
            Self::I32(_) => FieldKind::I32,
            Self::I64(_) => FieldKind::I64,
            Self::Bool(_) => FieldKind::Bool,
            Self::String(_) => FieldKind::String,
            Self::Vec2(_) => FieldKind::Vec2,
            Self::Vec3(_) => FieldKind::Vec3,
            Self::Vec4(_) => FieldKind::Vec4,
            Self::Quat(_) => FieldKind::Quat,
            Self::Mat4(_) => FieldKind::Mat4,
            Self::AssetRef { .. } => FieldKind::AssetRef,
            Self::EntityRef(_) => FieldKind::EntityRef,
        }
    }
}

impl fmt::Display for ReflectValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::F32(v) => write!(f, "{v}"),
            Self::F64(v) => write!(f, "{v}"),
            Self::U8(v) => write!(f, "{v}"),
            Self::U16(v) => write!(f, "{v}"),
            Self::U32(v) => write!(f, "{v}"),
            Self::U64(v) => write!(f, "{v}"),
            Self::I8(v) => write!(f, "{v}"),
            Self::I16(v) => write!(f, "{v}"),
            Self::I32(v) => write!(f, "{v}"),
            Self::I64(v) => write!(f, "{v}"),
            Self::Bool(v) => write!(f, "{v}"),
            Self::String(v) => write!(f, "{v}"),
            Self::Vec2(v) => write!(f, "({}, {})", v.x, v.y),
            Self::Vec3(v) => write!(f, "({}, {}, {})", v.x, v.y, v.z),
            Self::Vec4(v) => write!(f, "({}, {}, {}, {})", v.x, v.y, v.z, v.w),
            Self::Quat(v) => write!(f, "({}, {}, {}, {})", v.x, v.y, v.z, v.w),
            Self::Mat4(_) => write!(f, "[Mat4]"),
            Self::AssetRef { guid, asset_type } => match guid {
                Some(g) => write!(f, "{asset_type}({g})"),
                None => write!(f, "{asset_type}(none)"),
            },
            Self::EntityRef(Some(r)) => write!(f, "{r}"),
            Self::EntityRef(None) => write!(f, "(none)"),
        }
    }
}
