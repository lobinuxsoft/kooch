//! One scene open at a time — the behaviour that predates #609 and
//! must keep working.

use crate::allocator::EntityAllocator;
use crate::archetype_registry::ArchetypeRegistry;
use crate::commands::Commands;
use crate::component::{Component, ComponentRegistry};
use crate::ephemeral::EphemeralComponents;
use crate::query::{AccessTracker, Query};
use crate::reflect::{FieldKind, FieldMeta, Reflect, ReflectError, ReflectValue};
use crate::scene_manager::*;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Health {
    pub(super) hp: u32,
}

impl Component for Health {}

impl Reflect for Health {
    fn reflect_fields(&self) -> &'static [FieldMeta] {
        static FIELDS: &[FieldMeta] = &[FieldMeta {
            name: "hp",
            type_name: "u32",
            kind: FieldKind::U32,
            choices: &[],
            bits: &[],
            shown_when: None,
            asset_type: "",
            requires: "",
            doc: "",
        }];
        FIELDS
    }

    fn reflect_get(&self, field: &str) -> Option<ReflectValue> {
        match field {
            "hp" => Some(ReflectValue::U32(self.hp)),
            _ => None,
        }
    }

    fn reflect_set(&mut self, field: &str, value: ReflectValue) -> Result<(), ReflectError> {
        match (field, value) {
            ("hp", ReflectValue::U32(v)) => {
                self.hp = v;
                Ok(())
            }
            (other, _) => Err(ReflectError::FieldNotFound(other.into())),
        }
    }

    fn reflect_default() -> Self {
        Health { hp: 100 }
    }
}

struct Ephemeral;
impl Component for Ephemeral {}

pub(super) fn setup_resources() -> Resources {
    let mut resources = Resources::new();
    resources.insert(EntityAllocator::new());
    resources.insert(ComponentRegistry::new());
    resources.insert(ArchetypeRegistry::new());
    resources.insert(AccessTracker::new());
    resources.insert(Commands::new());
    resources.insert(EphemeralComponents::new());
    resources
        .get_mut::<ComponentRegistry>()
        .unwrap()
        .register_cpu_reflected::<Health>();
    resources
}

pub(super) fn tmp_path(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("kooch_scene_manager_test");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

#[test]
fn save_as_then_load_round_trips() {
    let mut resources = setup_resources();

    // Spawn a single Health entity.
    {
        let mut commands = resources.remove::<Commands>().unwrap();
        commands
            .spawn(&mut resources)
            .insert_reflected(Health { hp: 42 });
        commands.apply(&mut resources);
        resources.insert(commands);
    }

    let mut sm = SceneManager::new();
    let path = tmp_path("round_trip.scene");
    sm.save_as(path.clone(), &mut resources).unwrap();

    assert_eq!(sm.current(), Some(path.as_path()));
    assert!(!sm.is_dirty());

    // Mutate the live ECS so we can verify load wipes it.
    {
        let mut commands = resources.remove::<Commands>().unwrap();
        commands
            .spawn(&mut resources)
            .insert_reflected(Health { hp: 999 });
        commands.apply(&mut resources);
        resources.insert(commands);
    }

    sm.mark_dirty();
    sm.load(&path, &mut resources).unwrap();

    assert!(!sm.is_dirty(), "load must clear dirty flag");

    let query = Query::<&Health>::new(&resources);
    let healths: Vec<u32> = query.iter().map(|h| h.hp).collect();
    assert_eq!(healths, vec![42]);
}

#[test]
fn save_without_current_returns_not_found() {
    let mut resources = setup_resources();
    let mut sm = SceneManager::new();
    let err = sm
        .save(&mut resources)
        .expect_err("save with no current should fail");
    match err {
        SceneError::Io(e) => assert_eq!(e.kind(), std::io::ErrorKind::NotFound),
        other => panic!("expected Io(NotFound), got {other:?}"),
    }
}

#[test]
fn load_preserves_ephemeral_entities() {
    let mut resources = setup_resources();

    // Mark Ephemeral as ephemeral and spawn one of each.
    resources
        .get_mut::<EphemeralComponents>()
        .unwrap()
        .insert(std::any::TypeId::of::<Ephemeral>());

    {
        let mut commands = resources.remove::<Commands>().unwrap();
        commands
            .spawn(&mut resources)
            .insert_reflected(Health { hp: 1 });
        commands
            .spawn(&mut resources)
            .insert_reflected(Health { hp: 99 })
            .insert(Ephemeral);
        commands.apply(&mut resources);
        resources.insert(commands);
    }

    // Save a scene containing only the persistent entity (hp=1).
    let mut sm = SceneManager::new();
    let path = tmp_path("ephemeral.scene");
    sm.save_as(path.clone(), &mut resources).unwrap();

    // Reload — ephemeral hp=99 entity must survive the swap.
    sm.load(&path, &mut resources).unwrap();

    let query = Query::<&Health>::new(&resources);
    let mut healths: Vec<u32> = query.iter().map(|h| h.hp).collect();
    healths.sort();
    assert_eq!(healths, vec![1, 99]);
}

#[test]
fn dirty_lifecycle() {
    let mut sm = SceneManager::new();
    assert!(!sm.is_dirty());
    sm.mark_dirty();
    assert!(sm.is_dirty());
    sm.mark_clean();
    assert!(!sm.is_dirty());
}
