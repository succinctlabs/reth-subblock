use itertools::Itertools;
use rkyv::{
    collections::util::{Entry, EntryAdapter},
    rancor::Fallible,
    ser::{Allocator, Writer},
    vec::{ArchivedVec, VecResolver},
    with::{ArchiveWith, DeserializeWith, SerializeWith},
    Archive, Deserialize, Place, Serialize,
};

use std::{
    collections::{HashMap, HashSet},
    hash::{BuildHasher, Hash},
};

/// rkyv as vec, except sorted so it's deterministic.
#[derive(Debug)]
pub struct AsSortedVec;

impl<K: Archive, V: Archive, H> ArchiveWith<HashMap<K, V, H>> for AsSortedVec {
    type Archived = ArchivedVec<Entry<K::Archived, V::Archived>>;
    type Resolver = VecResolver;

    fn resolve_with(
        field: &HashMap<K, V, H>,
        resolver: Self::Resolver,
        out: Place<Self::Archived>,
    ) {
        ArchivedVec::resolve_from_len(field.len(), resolver, out);
    }
}

impl<K, V, H, S> SerializeWith<HashMap<K, V, H>, S> for AsSortedVec
where
    K: Serialize<S> + Ord + PartialOrd,
    V: Serialize<S>,
    S: Fallible + Allocator + Writer + ?Sized,
{
    fn serialize_with(
        field: &HashMap<K, V, H>,
        serializer: &mut S,
    ) -> Result<Self::Resolver, S::Error> {
        ArchivedVec::serialize_from_iter(
            field
                .iter()
                .sorted_by(|a, b| a.0.cmp(b.0)) // This is the ONLY DIFFERENCE from AsVec in the WHOLE FILE
                .map(|(key, value)| EntryAdapter::<_, _, K, V>::new(key, value)),
            serializer,
        )
    }
}

impl<K, V, H, D> DeserializeWith<ArchivedVec<Entry<K::Archived, V::Archived>>, HashMap<K, V, H>, D>
    for AsSortedVec
where
    K: Archive + Hash + Eq,
    V: Archive,
    K::Archived: Deserialize<K, D>,
    V::Archived: Deserialize<V, D>,
    H: BuildHasher + Default,
    D: Fallible + ?Sized,
{
    fn deserialize_with(
        field: &ArchivedVec<Entry<K::Archived, V::Archived>>,
        deserializer: &mut D,
    ) -> Result<HashMap<K, V, H>, D::Error> {
        let mut result = HashMap::with_capacity_and_hasher(field.len(), H::default());
        for entry in field.iter() {
            result.insert(
                entry.key.deserialize(deserializer)?,
                entry.value.deserialize(deserializer)?,
            );
        }
        Ok(result)
    }
}

impl<T: Archive, H> ArchiveWith<HashSet<T, H>> for AsSortedVec {
    type Archived = ArchivedVec<T::Archived>;
    type Resolver = VecResolver;

    fn resolve_with(field: &HashSet<T, H>, resolver: Self::Resolver, out: Place<Self::Archived>) {
        ArchivedVec::resolve_from_len(field.len(), resolver, out);
    }
}

impl<T, H, S> SerializeWith<HashSet<T, H>, S> for AsSortedVec
where
    T: Serialize<S> + Ord + PartialOrd,
    S: Fallible + Allocator + Writer + ?Sized,
{
    fn serialize_with(
        field: &HashSet<T, H>,
        serializer: &mut S,
    ) -> Result<Self::Resolver, S::Error> {
        ArchivedVec::<T::Archived>::serialize_from_iter::<T, _, _>(
            field.iter().sorted(),
            serializer,
        )
    }
}

impl<T, H, D> DeserializeWith<ArchivedVec<T::Archived>, HashSet<T, H>, D> for AsSortedVec
where
    T: Archive + Hash + Eq,
    T::Archived: Deserialize<T, D>,
    H: BuildHasher + Default,
    D: Fallible + ?Sized,
{
    fn deserialize_with(
        field: &ArchivedVec<T::Archived>,
        deserializer: &mut D,
    ) -> Result<HashSet<T, H>, D::Error> {
        let mut result = HashSet::with_capacity_and_hasher(field.len(), H::default());
        for key in field.iter() {
            result.insert(key.deserialize(deserializer)?);
        }
        Ok(result)
    }
}
