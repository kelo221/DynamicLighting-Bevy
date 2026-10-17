//! Builds the cluster buffer: for every cell, the realtime lights whose sphere can reach it.

use bevy::math::{UVec3, Vec2, Vec3};

use super::gpu::{GPU_CLUSTER_HEADER_SIZE, GpuClusterHeader};
use super::grid::{GRID, Grid, Sphere};
use crate::light::{LightType, RealtimeViewer};
use crate::shared::{ChannelFlags, GpuLight};

/// Most light indices the buffer holds (2 MiB of them); a frame that needs more turns the grid
/// off and visits every realtime light.
pub const MAX_ENTRIES: usize = 1 << 20;

/// The realtime cluster buffer's contents.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClusterLists {
    pub header: GpuClusterHeader,
    /// Each cell's first entry and entry count, two words per cell, then the entries: realtime
    /// light indices (0 is the first realtime light), two 16-bit indices per word.
    pub words: Vec<u32>,
    /// Light indices over all cells.
    pub entries: usize,
}

/// Why the grid is off this frame; every fragment then visits every realtime light.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridOff {
    NoCamera,
    FewLights,
    NothingInFront,
    TooManyEntries,
}

impl ClusterLists {
    /// The buffer contents, padded with zero words to a multiple of 16 bytes. WGSL sizes a
    /// struct ending in a runtime-sized array as at least one element, rounded up to the
    /// struct's 16-byte alignment, and rejects smaller bindings.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = bytemuck::bytes_of(&self.header).to_vec();
        bytes.extend_from_slice(bytemuck::cast_slice(&self.words));
        let padded = bytes
            .len()
            .max(GPU_CLUSTER_HEADER_SIZE + 4)
            .next_multiple_of(16);
        bytes.resize(padded, 0);
        bytes
    }

    /// The realtime lights a cell lists.
    pub fn lights_in(&self, cell: u32) -> impl Iterator<Item = u32> + '_ {
        let first = self.words[2 * cell as usize];
        let count = self.words[2 * cell as usize + 1];
        let base = self.header.entry_base as usize;
        (first..first + count).map(move |entry| {
            let word = self.words[base + (entry >> 1) as usize];
            (word >> ((entry & 1) * 16)) & 0xffff
        })
    }
}

/// How far a packed light reaches from its position, or `None` for a disabled record. Mirrors
/// `sqrt(radius_sqr) + light_extent(light)` in WESL.
pub fn light_sphere(light: &GpuLight) -> Option<Sphere> {
    if light.radius_sqr < 0.0 || light.radius_sqr.is_nan() {
        return None;
    }
    let kind = ChannelFlags::unpack(light.channel).light_type;
    let area = kind == LightType::Rectangle as u32 || kind == LightType::Tube as u32;
    let extent = if area {
        Vec2::new(light.gp1, light.gp2).length()
    } else {
        0.0
    };
    Some(Sphere {
        center: Vec3::from_array(light.position),
        radius: light.radius_sqr.sqrt() + extent,
    })
}

/// The cluster lists for the realtime lights `lights` (in buffer order) as seen by `viewer`.
pub fn build(
    viewer: Option<RealtimeViewer>,
    lights: &[GpuLight],
    min_lights: u32,
) -> Result<ClusterLists, GridOff> {
    let viewer = viewer.ok_or(GridOff::NoCamera)?;
    if lights.len() < min_lights.max(1) as usize || lights.len() > usize::from(u16::MAX) + 1 {
        return Err(GridOff::FewLights);
    }
    let spheres: Vec<Option<Sphere>> = lights.iter().map(light_sphere).collect();
    let grid =
        Grid::new(viewer, spheres.iter().flatten().copied()).ok_or(GridOff::NothingInFront)?;
    let reach: Vec<_> = spheres
        .iter()
        .map(|sphere| sphere.and_then(|sphere| grid.cells_touched(sphere)))
        .collect();

    let cell_count = GRID.element_product() as usize;
    let mut counts = vec![0u32; cell_count];
    let mut entries = 0usize;
    for (low, high) in reach.iter().flatten() {
        let cells = (*high - *low + 1).element_product() as usize;
        entries += cells;
        if entries > MAX_ENTRIES {
            return Err(GridOff::TooManyEntries);
        }
        for_each_cell(*low, *high, |cell| counts[cell] += 1);
    }

    let entry_base = 2 * cell_count;
    let mut words = vec![0u32; entry_base + entries.div_ceil(2)];
    let mut next = vec![0u32; cell_count];
    let mut first = 0u32;
    for (cell, &count) in counts.iter().enumerate() {
        words[2 * cell] = first;
        words[2 * cell + 1] = count;
        next[cell] = first;
        first += count;
    }
    for (index, cells) in reach.iter().enumerate() {
        let Some((low, high)) = cells else {
            continue;
        };
        for_each_cell(*low, *high, |cell| {
            let entry = next[cell];
            next[cell] += 1;
            words[entry_base + (entry >> 1) as usize] |= (index as u32) << ((entry & 1) * 16);
        });
    }
    Ok(ClusterLists {
        header: grid.header(entry_base as u32),
        words,
        entries,
    })
}

fn for_each_cell(low: UVec3, high: UVec3, mut visit: impl FnMut(usize)) {
    for z in low.z..=high.z {
        for y in low.y..=high.y {
            let row = (z * GRID.y + y) * GRID.x;
            for x in low.x..=high.x {
                visit((row + x) as usize);
            }
        }
    }
}
