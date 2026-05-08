use crate::allocator::EntityAllocator;
use crate::archetype_registry::ArchetypeRegistry;
use crate::commands::Commands;
use crate::component::{Component, ComponentRegistry};
use crate::query::AccessTracker;
use crate::reflect::{FieldKind, FieldMeta, Reflect, ReflectError, ReflectValue};
use ome_core::resource::Resources;

mod document;
mod sync;

// -- Test component with manual Reflect impl ----------------------------

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Health {
    pub hp: u32,
    pub max_hp: u32,
}

impl Component for Health {}

impl Reflect for Health {
    fn reflect_fields(&self) -> &'static [FieldMeta] {
        static FIELDS: &[FieldMeta] = &[
            FieldMeta {
                name: "hp",
                type_name: "u32",
                kind: FieldKind::U32,
                choices: &[],
                asset_type: "",
            },
            FieldMeta {
                name: "max_hp",
                type_name: "u32",
                kind: FieldKind::U32,
                choices: &[],
                asset_type: "",
            },
        ];
        FIELDS
    }

    fn reflect_get(&self, field: &str) -> Option<ReflectValue> {
        match field {
            "hp" => Some(ReflectValue::U32(self.hp)),
            "max_hp" => Some(ReflectValue::U32(self.max_hp)),
            _ => None,
        }
    }

    fn reflect_set(&mut self, field: &str, value: ReflectValue) -> Result<(), ReflectError> {
        match field {
            "hp" => match value {
                ReflectValue::U32(v) => {
                    self.hp = v;
                    Ok(())
                }
                other => Err(ReflectError::TypeMismatch {
                    field: "hp".into(),
                    expected: FieldKind::U32,
                    got: other.kind(),
                }),
            },
            "max_hp" => match value {
                ReflectValue::U32(v) => {
                    self.max_hp = v;
                    Ok(())
                }
                other => Err(ReflectError::TypeMismatch {
                    field: "max_hp".into(),
                    expected: FieldKind::U32,
                    got: other.kind(),
                }),
            },
            _ => Err(ReflectError::FieldNotFound(field.into())),
        }
    }

    fn reflect_default() -> Self {
        Health {
            hp: 100,
            max_hp: 100,
        }
    }
}

/// Helper: set up a minimal ECS with `Commands`, `ComponentRegistry`,
/// `ArchetypeRegistry`, `EntityAllocator`, and `AccessTracker`.
pub(super) fn setup_resources() -> Resources {
    let mut resources = Resources::new();
    resources.insert(EntityAllocator::new());
    resources.insert(ComponentRegistry::new());
    resources.insert(ArchetypeRegistry::new());
    resources.insert(AccessTracker::new());
    resources.insert(Commands::new());
    resources
}

// -- Marker component used by the ephemeral-filter tests ---------------

/// Zero-sized marker registered as ephemeral in the tests below.
pub(super) struct TestEphemeral;
impl Component for TestEphemeral {}

/// Component with the new typed asset reference fields. Mirrors the
/// shape of `MeshRenderer.mesh` / `.material` without pulling
/// `ome_render` into the test scope (would create a dep cycle).
#[derive(Default, Clone, Debug, PartialEq, ome_ecs_macros::Reflect)]
pub(super) struct TestAssetHolder {
    #[reflect(asset = "test::FakeMesh")]
    pub mesh: Option<ome_core::Guid>,
    #[reflect(asset = "test::FakeMaterial")]
    pub material: Option<ome_core::Guid>,
}

impl Component for TestAssetHolder {}
