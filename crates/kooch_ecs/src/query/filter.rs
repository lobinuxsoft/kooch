//! Query filters for archetype-level filtering.
//!
//! Filters determine which archetypes match a query based on component
//! presence/absence without actually fetching the component data.

use std::any::TypeId;
use std::marker::PhantomData;

use crate::archetype::Archetype;

/// Trait for query filters that operate at the archetype level.
pub trait QueryFilter {
    /// Returns the component `TypeId`s that the archetype MUST contain.
    fn required_ids() -> Vec<TypeId> {
        Vec::new()
    }

    /// Returns the component `TypeId`s that the archetype MUST NOT contain.
    fn excluded_ids() -> Vec<TypeId> {
        Vec::new()
    }

    /// Returns `true` if the archetype passes this filter.
    fn matches_archetype(archetype: &Archetype) -> bool;
}

/// No-op filter that matches all archetypes.
impl QueryFilter for () {
    fn matches_archetype(_archetype: &Archetype) -> bool {
        true
    }
}

/// Filter that requires the archetype to contain component `T`.
///
/// Does not fetch `T` — only checks for its presence.
pub struct With<T: 'static>(PhantomData<T>);

impl<T: 'static> QueryFilter for With<T> {
    fn required_ids() -> Vec<TypeId> {
        vec![TypeId::of::<T>()]
    }

    fn matches_archetype(archetype: &Archetype) -> bool {
        archetype.has_component::<T>()
    }
}

/// Filter that requires the archetype to NOT contain component `T`.
pub struct Without<T: 'static>(PhantomData<T>);

impl<T: 'static> QueryFilter for Without<T> {
    fn excluded_ids() -> Vec<TypeId> {
        vec![TypeId::of::<T>()]
    }

    fn matches_archetype(archetype: &Archetype) -> bool {
        !archetype.has_component::<T>()
    }
}

/// Combines two filters with AND semantics.
impl<A: QueryFilter, B: QueryFilter> QueryFilter for (A, B) {
    fn required_ids() -> Vec<TypeId> {
        let mut ids = A::required_ids();
        ids.extend(B::required_ids());
        ids
    }

    fn excluded_ids() -> Vec<TypeId> {
        let mut ids = A::excluded_ids();
        ids.extend(B::excluded_ids());
        ids
    }

    fn matches_archetype(archetype: &Archetype) -> bool {
        A::matches_archetype(archetype) && B::matches_archetype(archetype)
    }
}

/// Combines three filters with AND semantics.
impl<A: QueryFilter, B: QueryFilter, C: QueryFilter> QueryFilter for (A, B, C) {
    fn required_ids() -> Vec<TypeId> {
        let mut ids = A::required_ids();
        ids.extend(B::required_ids());
        ids.extend(C::required_ids());
        ids
    }

    fn excluded_ids() -> Vec<TypeId> {
        let mut ids = A::excluded_ids();
        ids.extend(B::excluded_ids());
        ids.extend(C::excluded_ids());
        ids
    }

    fn matches_archetype(archetype: &Archetype) -> bool {
        A::matches_archetype(archetype)
            && B::matches_archetype(archetype)
            && C::matches_archetype(archetype)
    }
}

#[cfg(test)]
mod tests;
