//! The header of the realtime cluster buffer. The WESL `ClusterHeader` must match.

use bytemuck::{Pod, Zeroable};

/// Size in bytes of [`GpuClusterHeader`]; the buffer's words follow it.
pub const GPU_CLUSTER_HEADER_SIZE: usize = 128;

/// How the shader finds a fragment's cell. All zero, the grid is off and every fragment
/// visits every realtime light.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuClusterHeader {
    /// The clustering camera's projection times its view, column-major.
    pub clip_from_world: [[f32; 4]; 4],
    /// Distance in front of the camera: `dot(depth_row.xyz, world) + depth_row.w`.
    pub depth_row: [f32; 4],
    /// Cells across, down and deep; zero when the grid is off.
    pub dims: [u32; 3],
    /// The word where the light indices start: two words per cell come first.
    pub entry_base: u32,
    /// Where the first depth slice ends; the slices behind it are spaced exponentially. Unused by
    /// orthographic cameras.
    pub first_depth: f32,
    /// No realtime light reaches past this depth.
    pub far: f32,
    /// Depth slice coordinate: `log2(depth) * scale + bias`, plus one for the first slice, for
    /// perspective cameras; `depth * scale + bias` for orthographic ones.
    pub slice_scale: f32,
    pub slice_bias: f32,
    /// `FLAG_*` bits.
    pub flags: u32,
    pub _padding: [u32; 3],
}

impl GpuClusterHeader {
    /// The camera has a perspective projection, so depth slices are spaced exponentially.
    pub const FLAG_PERSPECTIVE: u32 = 1;
}

#[cfg(test)]
mod tests {
    use core::mem::{offset_of, size_of};

    use super::*;
    use crate::light_clusters::shader_tests::compile_clusters;
    use crate::shared::wesl_check::struct_layout;

    #[test]
    fn header_layout_matches_wesl() {
        assert_eq!(size_of::<GpuClusterHeader>(), GPU_CLUSTER_HEADER_SIZE);
        let rust = [
            (
                "clip_from_world",
                offset_of!(GpuClusterHeader, clip_from_world),
            ),
            ("depth_row", offset_of!(GpuClusterHeader, depth_row)),
            ("dims", offset_of!(GpuClusterHeader, dims)),
            ("entry_base", offset_of!(GpuClusterHeader, entry_base)),
            ("first_depth", offset_of!(GpuClusterHeader, first_depth)),
            ("far", offset_of!(GpuClusterHeader, far)),
            ("slice_scale", offset_of!(GpuClusterHeader, slice_scale)),
            ("slice_bias", offset_of!(GpuClusterHeader, slice_bias)),
            ("flags", offset_of!(GpuClusterHeader, flags)),
        ];
        let module = compile_clusters();
        let wesl = struct_layout(&module, "ClusterHeader");
        assert_eq!(wesl.size as usize, GPU_CLUSTER_HEADER_SIZE);
        let wesl_members: Vec<(&str, usize)> = wesl
            .members
            .iter()
            .map(|(name, offset)| (name.as_str(), *offset as usize))
            .collect();
        assert_eq!(wesl_members, rust);
    }

    #[test]
    fn words_follow_the_header() {
        let module = compile_clusters();
        let buffer = struct_layout(&module, "RealtimeClusters");
        assert_eq!(
            buffer.members,
            vec![
                ("header".to_string(), 0),
                ("words".to_string(), GPU_CLUSTER_HEADER_SIZE as u32)
            ]
        );
        // The shader's minimum binding size: what an off grid uploads.
        let off = crate::light_clusters::ClusterLists::default().to_bytes();
        assert_eq!(off.len(), buffer.size as usize);
    }

    #[test]
    fn flags_match_wesl() {
        let source = crate::light_clusters::shader_tests::CLUSTERS.1;
        let line = format!(
            "const CLUSTER_FLAG_PERSPECTIVE: u32 = {}u;",
            GpuClusterHeader::FLAG_PERSPECTIVE
        );
        assert!(source.contains(&line), "{line} is missing");
    }
}
