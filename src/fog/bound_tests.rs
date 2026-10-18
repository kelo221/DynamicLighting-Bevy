//! The culling sphere of every fog shape holds everything the fog shader draws: rays that pass
//! outside it see no fog in the shader's CPU mirror.

use bevy::math::{Quat, Vec2, Vec3};

use super::mirror::{FogVolume, opacity};
use super::*;
use crate::light::DynamicLight;
use crate::shared::GpuLight;

const SHAPES: [VolumetricShape; 5] = [
    VolumetricShape::Sphere,
    VolumetricShape::Box,
    VolumetricShape::ConeZ,
    VolumetricShape::ConeY,
    VolumetricShape::Frustum,
];
const CUTOFFS: [f32; 11] = [
    1.0, 15.0, 30.0, 45.0, 60.0, 75.0, 89.0, 90.0, 120.0, 147.5, 180.0,
];

/// The volume the fog shader reads from `record`, following `fog::pack`'s layout.
fn rendered(record: &GpuLight, shape: VolumetricShape) -> FogVolume {
    FogVolume {
        shape,
        center: record.position.into(),
        axis: record.forward.into(),
        up: record.up.into(),
        scale: Vec3::new(record.gp2, record.gp3, record.shimmer_scale),
        half: Vec2::new(record.gp2, record.gp3),
        slope: record.shimmer_scale,
        radius: record.radius_sqr,
        thickness: record.gp1,
        intensity: record.volumetric_intensity,
        inverse_visibility: record.volumetric_visibility,
        cone_term: record.gp2,
    }
}

/// Evenly spread unit directions (a golden-angle spiral).
fn directions(count: usize) -> impl Iterator<Item = Vec3> {
    (0..count).map(move |index| {
        let z = 1.0 - 2.0 * (index as f32 + 0.5) / count as f32;
        let angle = index as f32 * 2.399_963;
        let ring = (1.0 - z * z).max(0.0).sqrt();
        Vec3::new(ring * angle.cos(), ring * angle.sin(), z)
    })
}

/// Segments along many directions, each passing `distance` from `center` at its closest.
fn segments_at(center: Vec3, distance: f32) -> impl Iterator<Item = (Vec3, Vec3)> {
    directions(48).flat_map(move |along| {
        directions(24).filter_map(move |offset| {
            let side = offset.reject_from_normalized(along).try_normalize()?;
            let closest = center + side * distance;
            let reach = along * distance * 3.0;
            Some((closest - reach, closest + reach))
        })
    })
}

fn placement() -> Placement {
    Placement {
        position: Vec3::new(2.0, -1.0, 4.0),
        rotation: Quat::from_euler(bevy::math::EulerRot::YXZ, 0.7, -0.4, 0.2),
        scale: Vec3::new(0.5, 1.5, 2.0),
    }
}

fn light(outer_cutoff: f32) -> DynamicLight {
    DynamicLight {
        outer_cutoff,
        inner_cutoff: 0.0,
        ..DynamicLight::rectangle(1.0, 0.5)
    }
}

/// Whether any of the segments sees fog.
fn sees_fog(volume: &FogVolume, segments: impl Iterator<Item = (Vec3, Vec3)>) -> bool {
    segments
        .into_iter()
        .any(|(start, end)| opacity(volume, start, end) > 0.0)
}

#[test]
fn no_fog_is_drawn_outside_the_culling_sphere() {
    let placement = placement();
    for shape in SHAPES {
        let fog = VolumetricLight {
            shape,
            ..Default::default()
        };
        for cutoff in CUTOFFS {
            let light = light(cutoff);
            let record = pack(&light, &fog, 1.0, placement);
            let volume = rendered(&record, shape);
            let bound = bounding_radius(&fog, &light, placement.scale);
            let outside = segments_at(placement.position, bound * 1.001 + 1.0e-4);
            assert!(!sees_fog(&volume, outside), "{shape:?} at {cutoff}°");
            // The sphere isn't trivially large: rays through its middle see fog. At 180° the
            // cone term is zero and the cone narrows to a line.
            let cone = matches!(shape, VolumetricShape::ConeZ | VolumetricShape::ConeY);
            if !(cone && record.gp2.abs() < 0.05) {
                let inside = segments_at(placement.position, bound * 0.25);
                assert!(sees_fog(&volume, inside), "{shape:?} at {cutoff}° is empty");
            }
        }
    }
}

#[test]
fn the_cone_rim_lies_past_the_cutoffs_own_sphere() {
    // At 30° the shader's cone opens to 0.55 rad, so its rim lies 4 / cos 0.55 ≈ 4.692 m from the
    // tip, past the 4 / cos 30° ≈ 4.619 m a sphere from the cutoff itself would give.
    let fog = VolumetricLight {
        shape: VolumetricShape::ConeZ,
        ..Default::default()
    };
    let placement = Placement {
        scale: Vec3::ONE,
        ..placement()
    };
    let light = light(30.0);
    let volume = rendered(&pack(&light, &fog, 1.0, placement), fog.shape);
    let by_cutoff = 4.0 / 30.0_f32.to_radians().cos();
    assert!(sees_fog(
        &volume,
        segments_at(placement.position, by_cutoff + 0.02)
    ));
    assert!(bounding_radius(&fog, &light, placement.scale) > by_cutoff + 0.07);
}
