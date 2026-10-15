//! Stable identity for lights and meshes across runs.

use bevy::prelude::*;

/// Identifies a light or mesh in bake data.
///
/// Entity ids change between runs, so bake data is matched by `BakeId` (plus a content hash for
/// meshes), never by entity. Sorting by `BakeId` gives every bake step a stable order.
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[reflect(Component, Debug, PartialEq, Hash, Clone)]
pub struct BakeId(pub u64);

impl BakeId {
    /// Derives an id from a path of names, e.g. the `Name`s from the scene root down to the
    /// entity. Segments are length-prefixed, so `["ab", "c"]` and `["a", "bc"]` differ.
    pub fn from_path<'a>(segments: impl IntoIterator<Item = &'a str>) -> Self {
        let mut hash = Fnv1a64::new();
        for segment in segments {
            hash.write(&(segment.len() as u64).to_le_bytes());
            hash.write(segment.as_bytes());
        }
        Self(hash.finish())
    }

    /// The id of part `index` of something baked as several entries, such as the shadow points
    /// of an area light.
    pub fn part(self, index: u32) -> Self {
        let mut hash = Fnv1a64::new();
        hash.write(b"part");
        hash.write(&self.0.to_le_bytes());
        hash.write(&index.to_le_bytes());
        Self(hash.finish())
    }
}

/// The id bake data knows a baked mesh or light by: its explicit [`BakeId`], or one derived from
/// its `Name` path exactly as the bake derives it. Kept up to date by the bake slice; the runtime
/// matches bake data with this, not with [`BakeId`].
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ResolvedBakeId(pub BakeId);

/// 64-bit FNV-1a: tiny, dependency-free and identical on every platform and run.
struct Fnv1a64(u64);

impl Fnv1a64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    fn new() -> Self {
        Self(Self::OFFSET_BASIS)
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 ^= u64::from(byte);
            self.0 = self.0.wrapping_mul(Self::PRIME);
        }
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fnv(bytes: &[u8]) -> u64 {
        let mut hash = Fnv1a64::new();
        hash.write(bytes);
        hash.finish()
    }

    #[test]
    fn fnv_matches_published_test_vectors() {
        assert_eq!(fnv(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn paths_are_stable_and_unambiguous() {
        let lamp = BakeId::from_path(["Level", "Hall", "Lamp"]);
        assert_eq!(lamp, BakeId::from_path(["Level", "Hall", "Lamp"]));
        assert_ne!(lamp, BakeId::from_path(["Level", "Hall", "Lamp2"]));
        assert_ne!(
            BakeId::from_path(["ab", "c"]),
            BakeId::from_path(["a", "bc"])
        );
        assert_eq!(BakeId::from_path([]), BakeId(Fnv1a64::OFFSET_BASIS));
    }

    #[test]
    fn parts_are_stable_and_distinct() {
        let lamp = BakeId::from_path(["Lamp"]);
        assert_eq!(lamp.part(1), lamp.part(1));
        let ids = [lamp, lamp.part(0), lamp.part(1), BakeId(lamp.0 + 1).part(0)];
        for (i, a) in ids.iter().enumerate() {
            for b in &ids[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn ids_sort_by_value() {
        let mut ids = [BakeId(3), BakeId(1), BakeId(2)];
        ids.sort();
        assert_eq!(ids, [BakeId(1), BakeId(2), BakeId(3)]);
    }
}
