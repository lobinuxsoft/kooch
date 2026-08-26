use std::marker::PhantomData;

use crate::archetype::Archetype;
use crate::archetype_registry::ArchetypeRegistry;
use crate::storage::Table;

use crate::query::fetch::WorldQuery;
use crate::query::filter::QueryFilter;

/// Iterator over entities matching a [`Query`].
///
/// [`Query`]: super::Query
pub struct QueryIter<'w, 'q, Q: WorldQuery, F: QueryFilter> {
    pub(super) fetch: &'q Q::Fetch<'w>,
    pub(super) archetypes: &'q [&'w Archetype],
    /// The table behind each archetype, parallel to `archetypes`.
    ///
    /// 🔴 Threaded through even though every entry is `None` today. This
    /// iterator and `for_each` must read the same place: if one followed
    /// the column and the other did not, an entity whose values had moved
    /// would be visible to half the engine and absent from the other half,
    /// with nothing failing. See #891.
    pub(super) tables: &'q [Option<&'w Table>],
    pub(super) registry: &'w ArchetypeRegistry,
    pub(super) archetype_idx: usize,
    pub(super) entity_idx: usize,
    pub(super) _marker: PhantomData<F>,
}

impl<'w, 'q, Q: WorldQuery, F: QueryFilter> Iterator for QueryIter<'w, 'q, Q, F> {
    type Item = Q::Item<'w>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.archetype_idx >= self.archetypes.len() {
                return None;
            }

            let archetype = self.archetypes[self.archetype_idx];
            let table = self.tables[self.archetype_idx];
            let entities = archetype.entities();

            if self.entity_idx >= entities.len() {
                self.archetype_idx += 1;
                self.entity_idx = 0;
                continue;
            }

            let entity = entities[self.entity_idx];
            self.entity_idx += 1;

            // SAFETY: Archetype matching guarantees required components exist.
            // The fetch was properly initialised with valid borrows.
            let at = table.and_then(|table| Some((table, self.registry.row_of(entity)?)));
            if let Some(item) = unsafe { Q::fetch(self.fetch, entity, at) } {
                return Some(item);
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining: usize = self.archetypes[self.archetype_idx..]
            .iter()
            .map(|a| a.entities().len())
            .sum::<usize>()
            .saturating_sub(self.entity_idx);
        (0, Some(remaining))
    }
}
