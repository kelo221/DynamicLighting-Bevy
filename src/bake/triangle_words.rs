//! Henry's `DynamicTrianglesBuilder.BuildDynamicTrianglesData`: one mesh's `dynamic_triangles`
//! words.
//!
//! ```text
//! [0 .. 4·T)   per triangle: lightDataOffset, boundsX − 2, boundsY − 2, boundsW + 4
//! light data   per triangle: count, then per light: lightIndex, shadowOffset[, bounceOffset]
//! payload      per triangle, per light: shadow bits, then bounce words (bounce layout only)
//! ```
//!
//! Offsets are relative to the start of the mesh's words; 0 means "no data". `boundsX − 2` and
//! `boundsY − 2` wrap below zero like Henry's `uint` math. `lightIndex` is the light's slot in
//! BVH order. The bounce layout (3 words per light) is used by every mesh when any light in the
//! scene bakes bounce lighting.

use super::bits::TexelRect;

/// Words per triangle header.
pub const TRIANGLE_HEADER_WORDS: usize = 4;

/// One light entry of a triangle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TriangleLight {
    /// The light's BVH slot.
    pub light: u32,
    /// Shadow bits over the padded crop rectangle; `None` stores offset 0.
    pub shadow: Option<Vec<u32>>,
    /// Packed bounce texels over the same rectangle; `None` stores offset 0.
    pub bounce: Option<Vec<u32>>,
}

/// One triangle's header data and lights.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TriangleRecord {
    /// The crop rectangle, before the 2 px padding.
    pub crop: TexelRect,
    pub lights: Vec<TriangleLight>,
}

impl TriangleRecord {
    /// Width of the stored (padded) shadow grid: `5 + maxX − minX`.
    pub fn padded_width(&self) -> u32 {
        self.crop.width() + 4
    }
}

/// Serializes a mesh's triangles. `bounce_layout` selects 3 words per light entry.
pub fn serialize_triangles(triangles: &[TriangleRecord], bounce_layout: bool) -> Vec<u32> {
    let stride = if bounce_layout { 3 } else { 2 };
    let light_entries: usize = triangles.iter().map(|t| t.lights.len()).sum();
    let mut words =
        Vec::with_capacity(triangles.len() * (TRIANGLE_HEADER_WORDS + 1) + light_entries * stride);

    for triangle in triangles {
        words.push(0);
        words.push(triangle.crop.min_x.wrapping_sub(2));
        words.push(triangle.crop.min_y.wrapping_sub(2));
        words.push(triangle.padded_width());
    }

    let mut light_data = Vec::with_capacity(triangles.len());
    for (index, triangle) in triangles.iter().enumerate() {
        let offset = words.len() as u32;
        words[index * TRIANGLE_HEADER_WORDS] = offset;
        light_data.push(offset as usize);
        words.push(triangle.lights.len() as u32);
        for light in &triangle.lights {
            words.push(light.light);
            words.push(0);
            if bounce_layout {
                words.push(0);
            }
        }
    }

    for (triangle, &light_data) in triangles.iter().zip(&light_data) {
        for (k, light) in triangle.lights.iter().enumerate() {
            let entry = light_data + 1 + k * stride;
            if let Some(shadow) = &light.shadow {
                words[entry + 1] = words.len() as u32;
                words.extend_from_slice(shadow);
            }
            if bounce_layout && let Some(bounce) = &light.bounce {
                words[entry + 2] = words.len() as u32;
                words.extend_from_slice(bounce);
            }
        }
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(min_x: u32, min_y: u32, max_x: u32, max_y: u32) -> TexelRect {
        TexelRect {
            min_x,
            min_y,
            max_x,
            max_y,
        }
    }

    #[test]
    fn layout_matches_henrys_builder() {
        let triangles = [
            TriangleRecord {
                crop: rect(4, 6, 8, 9),
                lights: vec![
                    TriangleLight {
                        light: 3,
                        shadow: Some(vec![0xAAAA, 0x1]),
                        bounce: None,
                    },
                    TriangleLight {
                        light: 7,
                        shadow: Some(vec![0xFF]),
                        bounce: None,
                    },
                ],
            },
            TriangleRecord {
                crop: rect(0, 1, 2, 2),
                lights: vec![],
            },
        ];
        let words = serialize_triangles(&triangles, false);
        #[rustfmt::skip]
        let expected = [
            8, 2, 4, 9,                       // header 0: width 5 + 8 - 4
            13, u32::MAX - 1, u32::MAX, 7,    // header 1: bounds wrap below zero
            2, 3, 14, 7, 16,                  // light data 0
            0,                                // light data 1: no lights
            0xAAAA, 0x1,                      // shadow bits of light 3
            0xFF,                             // shadow bits of light 7
        ];
        assert_eq!(words, expected);
    }

    #[test]
    fn bounce_layout_adds_a_third_word() {
        let triangles = [TriangleRecord {
            crop: rect(2, 2, 3, 3),
            lights: vec![
                TriangleLight {
                    light: 0,
                    shadow: Some(vec![5]),
                    bounce: Some(vec![9, 10]),
                },
                TriangleLight {
                    light: 1,
                    shadow: None,
                    bounce: None,
                },
            ],
        }];
        let words = serialize_triangles(&triangles, true);
        assert_eq!(words, [4, 0, 0, 6, 2, 0, 11, 12, 1, 0, 0, 5, 9, 10]);
        // Without the bounce layout, bounce data is not stored at all.
        let words = serialize_triangles(&triangles, false);
        assert_eq!(words, [4, 0, 0, 6, 2, 0, 9, 1, 0, 5]);
    }
}
