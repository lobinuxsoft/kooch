//! `WorldQuery` trait and fetch implementations.
//!
//! Defines how query parameters resolve to concrete component data.
//! Each `WorldQuery` implementation knows which `TypeId`s it accesses,
//! whether access is mutable, and how to fetch data for a single entity.

use std::any::TypeId;

use crate::component::StorageId;
use crate::component::registry::ComponentRegistry;
use crate::component::traits::AnyStorage;
use crate::entity::Entity;
use crate::storage::{Column, Table, TableRow};

/// Where an entity's values live, when they live in a table.
///
/// `None` while the archetype's components are still in the per-type map
/// — which, during the migration of #891, is most of them. Resolved once
/// per archetype by the query rather than once per entity.
pub type Row<'w> = Option<(&'w Table, TableRow)>;

use super::access::AccessTracker;

/// Describes the access pattern for a single fetch parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessKind {
    /// Immutable component access.
    Read,
    /// Mutable component access.
    Write,
}

/// Describes one component access in a query.
#[derive(Clone, Debug)]
pub struct ComponentAccess {
    /// The component `TypeId` being accessed.
    pub type_id: TypeId,
    /// Whether the access is read or write.
    pub kind: AccessKind,
}

/// Defines how a query parameter fetches component data.
///
/// Implemented for `&T`, `&mut T`, `Entity`, `Option<&T>`, and tuples.
///
/// # Safety
///
/// Implementations that use raw pointers from `AnyStorage` must ensure the
/// pointer is cast to the correct concrete type (matching the `TypeId` used
/// during registration).
pub trait WorldQuery {
    /// The item yielded per entity during iteration.
    type Item<'w>;

    /// The state needed to fetch items, created once per query.
    type Fetch<'w>;

    /// Returns the component `TypeId`s this query reads or writes.
    fn component_access() -> Vec<ComponentAccess>;

    /// Returns the `TypeId`s that the archetype MUST contain for this
    /// query to match (excludes optional components).
    fn required_type_ids() -> Vec<TypeId>;

    /// Initialises the fetch state from the component registry.
    ///
    /// Registers borrows in the access tracker.
    ///
    /// # Safety
    ///
    /// Caller must ensure no conflicting borrows exist on the same TypeIds.
    unsafe fn init_fetch<'w>(
        registry: &'w ComponentRegistry,
        tracker: &'w AccessTracker,
    ) -> Self::Fetch<'w>;

    /// Releases borrows registered during `init_fetch`.
    fn release_fetch(tracker: &AccessTracker);

    /// Fetches the item for a single entity.
    ///
    /// Returns `None` if the entity doesn't have the required components.
    ///
    /// # Safety
    ///
    /// Caller must ensure the fetch was initialised and the entity has the
    /// required components (verified at the archetype level).
    unsafe fn fetch<'w>(
        fetch: &Self::Fetch<'w>,
        entity: Entity,
        at: Row<'w>,
    ) -> Option<Self::Item<'w>>;
}

// ---------------------------------------------------------------------------
// Entity — yields the Entity itself
// ---------------------------------------------------------------------------

impl WorldQuery for Entity {
    type Item<'w> = Entity;
    type Fetch<'w> = ();

    fn component_access() -> Vec<ComponentAccess> {
        Vec::new()
    }

    fn required_type_ids() -> Vec<TypeId> {
        Vec::new()
    }

    unsafe fn init_fetch<'w>(
        _registry: &'w ComponentRegistry,
        _tracker: &'w AccessTracker,
    ) -> Self::Fetch<'w> {
    }

    fn release_fetch(_tracker: &AccessTracker) {}

    unsafe fn fetch<'w>(
        _fetch: &Self::Fetch<'w>,
        entity: Entity,
        _at: Row<'w>,
    ) -> Option<Self::Item<'w>> {
        Some(entity)
    }
}

// ---------------------------------------------------------------------------
// Read<T> — immutable component access via &T syntax
// ---------------------------------------------------------------------------

/// Fetch state for immutable component access.
pub struct ReadFetch<'w, T: 'static> {
    storage: Option<&'w dyn AnyStorage>,
    /// Which column holds `T`, once its values have moved to one (#891).
    id: Option<StorageId>,
    _marker: std::marker::PhantomData<T>,
}

impl<T: Send + Sync + 'static> WorldQuery for &T {
    type Item<'w> = &'w T;
    type Fetch<'w> = ReadFetch<'w, T>;

    fn component_access() -> Vec<ComponentAccess> {
        vec![ComponentAccess {
            type_id: TypeId::of::<T>(),
            kind: AccessKind::Read,
        }]
    }

    fn required_type_ids() -> Vec<TypeId> {
        vec![TypeId::of::<T>()]
    }

    unsafe fn init_fetch<'w>(
        registry: &'w ComponentRegistry,
        tracker: &'w AccessTracker,
    ) -> Self::Fetch<'w> {
        let storage = unsafe { registry.storage(&TypeId::of::<T>()) };
        if storage.is_some() {
            tracker.borrow_read(TypeId::of::<T>());
        }
        ReadFetch {
            storage,
            id: registry.storage_id(&TypeId::of::<T>()),
            _marker: std::marker::PhantomData,
        }
    }

    fn release_fetch(tracker: &AccessTracker) {
        // Only release if we actually acquired the borrow.
        // If the type wasn't registered, borrow_read was never called.
        tracker.release_read(TypeId::of::<T>());
    }

    unsafe fn fetch<'w>(
        fetch: &Self::Fetch<'w>,
        entity: Entity,
        at: Row<'w>,
    ) -> Option<Self::Item<'w>> {
        // The column first, for components whose values have moved there.
        // 🔴 A value lives in exactly ONE place — moving it to a column
        // takes it out of the map — so this is a lookup, not a preference:
        // whichever holds it is where it is.
        if let Some((table, row)) = at
            && let Some(id) = fetch.id
            && let Some(column) = table.column(id)
        {
            // SAFETY: the column was built for the component `id` names,
            // which is `T`.
            return unsafe { column.get::<T>(row.index()) };
        }
        let storage = fetch.storage?;
        let ptr = storage.get_ptr(entity)?;
        // SAFETY: The storage was registered with TypeId::of::<T>(), so the
        // pointer points to valid T data. The lifetime is tied to 'w (the
        // storage reference lifetime).
        Some(unsafe { &*(ptr as *const T) })
    }
}

// ---------------------------------------------------------------------------
// Write<T> — mutable component access via &mut T syntax
// ---------------------------------------------------------------------------

/// Fetch state for mutable component access.
pub struct WriteFetch<'w, T: 'static> {
    storage: Option<&'w mut dyn AnyStorage>,
    /// Which column holds `T`, once its values have moved to one (#891).
    id: Option<StorageId>,
    _marker: std::marker::PhantomData<T>,
}

impl<T: Send + Sync + 'static> WorldQuery for &mut T {
    type Item<'w> = &'w mut T;
    type Fetch<'w> = WriteFetch<'w, T>;

    fn component_access() -> Vec<ComponentAccess> {
        vec![ComponentAccess {
            type_id: TypeId::of::<T>(),
            kind: AccessKind::Write,
        }]
    }

    fn required_type_ids() -> Vec<TypeId> {
        vec![TypeId::of::<T>()]
    }

    unsafe fn init_fetch<'w>(
        registry: &'w ComponentRegistry,
        tracker: &'w AccessTracker,
    ) -> Self::Fetch<'w> {
        let storage = unsafe { registry.storage_mut(&TypeId::of::<T>()) };
        if storage.is_some() {
            tracker.borrow_write(TypeId::of::<T>());
        }
        WriteFetch {
            storage,
            id: registry.storage_id(&TypeId::of::<T>()),
            _marker: std::marker::PhantomData,
        }
    }

    fn release_fetch(tracker: &AccessTracker) {
        tracker.release_write(TypeId::of::<T>());
    }

    unsafe fn fetch<'w>(
        fetch: &Self::Fetch<'w>,
        entity: Entity,
        at: Row<'w>,
    ) -> Option<Self::Item<'w>> {
        // 🔴 The write path has to follow the read path exactly. If `&T`
        // read a column and `&mut T` did not, an entity whose values had
        // moved would simply stop appearing in mutable queries — present
        // to one half of the engine and absent to the other.
        if let Some((table, row)) = at
            && let Some(id) = fetch.id
            && let Some(column) = table.column(id)
        {
            // SAFETY: the column was built for `T`, and the borrow
            // tracker guarantees this query holds the only access to this
            // component this frame.
            //
            // 🔴 Through the column's OWN pointer, never by casting the
            // shared borrow to `*mut` — Miri rejects that retag, and it is
            // right to: the shared tag does not grant writes.
            return unsafe { column.value_ptr::<T>(row.index()).map(|p| &mut *p) };
        }
        let storage = fetch.storage.as_ref()?;
        // SAFETY: We need a mutable pointer but fetch holds &mut dyn AnyStorage.
        // Each entity maps to a unique slot in the storage (different HashMap keys
        // or different Vec indices), so references to different entities never alias.
        // The borrow tracker ensures no other query accesses this storage.
        let storage_ptr = *storage as *const dyn AnyStorage as *mut dyn AnyStorage;
        let ptr = unsafe { (*storage_ptr).get_mut_ptr(entity)? };
        Some(unsafe { &mut *(ptr as *mut T) })
    }
}

// ---------------------------------------------------------------------------
// Option<Q> — optional component access
// ---------------------------------------------------------------------------

impl<Q: WorldQuery> WorldQuery for Option<Q> {
    type Item<'w> = Option<Q::Item<'w>>;
    type Fetch<'w> = Option<Q::Fetch<'w>>;

    fn component_access() -> Vec<ComponentAccess> {
        Q::component_access()
    }

    /// Optional components don't add to required TypeIds — the archetype
    /// doesn't need to contain them.
    fn required_type_ids() -> Vec<TypeId> {
        Vec::new()
    }

    unsafe fn init_fetch<'w>(
        registry: &'w ComponentRegistry,
        tracker: &'w AccessTracker,
    ) -> Self::Fetch<'w> {
        // Check if all required component types are registered before initialising.
        let all_registered = Q::component_access()
            .iter()
            .all(|a| registry.contains_type(&a.type_id));
        if all_registered {
            Some(unsafe { Q::init_fetch(registry, tracker) })
        } else {
            None
        }
    }

    fn release_fetch(_tracker: &AccessTracker) {
        // Release for Option is handled by the Query drop implementation,
        // which checks whether the inner fetch was actually initialised.
        // Calling Q::release_fetch here would be incorrect because we don't
        // know if init_fetch returned Some or None.
    }

    unsafe fn fetch<'w>(
        fetch: &Self::Fetch<'w>,
        entity: Entity,
        at: Row<'w>,
    ) -> Option<Self::Item<'w>> {
        match fetch {
            Some(inner) => Some(unsafe { Q::fetch(inner, entity, at) }),
            None => Some(None),
        }
    }
}

// ---------------------------------------------------------------------------
// Tuple implementations (up to 8 elements)
// ---------------------------------------------------------------------------

macro_rules! impl_world_query_tuple {
    ($($name:ident),+) => {
        impl<$($name: WorldQuery),+> WorldQuery for ($($name,)+) {
            type Item<'w> = ($($name::Item<'w>,)+);
            type Fetch<'w> = ($($name::Fetch<'w>,)+);

            fn component_access() -> Vec<ComponentAccess> {
                let mut access = Vec::new();
                $(access.extend($name::component_access());)+
                access
            }

            fn required_type_ids() -> Vec<TypeId> {
                let mut ids = Vec::new();
                $(ids.extend($name::required_type_ids());)+
                ids
            }

            #[allow(non_snake_case)]
            unsafe fn init_fetch<'w>(
                registry: &'w ComponentRegistry,
                tracker: &'w AccessTracker,
            ) -> Self::Fetch<'w> {
                ($(unsafe { $name::init_fetch(registry, tracker) },)+)
            }

            fn release_fetch(tracker: &AccessTracker) {
                $($name::release_fetch(tracker);)+
            }

            #[allow(non_snake_case)]
            unsafe fn fetch<'w>(
                fetch: &Self::Fetch<'w>,
                entity: Entity,
                at: Row<'w>,
            ) -> Option<Self::Item<'w>> {
                let ($($name,)+) = fetch;
                Some((
                    $(unsafe { $name::fetch($name, entity, at)? },)+
                ))
            }
        }
    };
}

impl_world_query_tuple!(A);
impl_world_query_tuple!(A, B);
impl_world_query_tuple!(A, B, C);
impl_world_query_tuple!(A, B, C, D);
impl_world_query_tuple!(A, B, C, D, E);
impl_world_query_tuple!(A, B, C, D, E, F);
impl_world_query_tuple!(A, B, C, D, E, F, G);
impl_world_query_tuple!(A, B, C, D, E, F, G, H);
