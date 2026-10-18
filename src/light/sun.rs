//! CPU reference for the sun math in `shaders/sun.wesl`, evaluated from the same [`GpuLight`]
//! record: a sun's `forward` points towards it, and `gp1`/`gp2` are the cosine and sine of its
//! disk's angular radius.

use bevy::math::Vec3;

use crate::shared::GpuLight;

/// The direction within the sun's disk closest to the mirror direction `reflection`: the
/// mirror direction itself when it points into the disk, otherwise the disk's edge towards it.
/// Specular light comes from there, as from an area light's representative point.
pub fn specular_direction(light: &GpuLight, reflection: Vec3) -> Vec3 {
    let toward = Vec3::from(light.forward);
    let along = toward.dot(reflection);
    if along >= light.gp1 {
        return reflection;
    }
    let side = reflection - toward * along;
    let side_length = side.length();
    if side_length < 1.0e-6 {
        return toward;
    }
    (toward * light.gp1 + side * (light.gp2 / side_length)).normalize()
}

/// Karis's energy normalization for [`specular_direction`]: a wider disk widens the highlight
/// without adding energy, (α / α′)² with α′ = saturate(α + sin θ / 3), as area lights do per axis.
pub fn specular_scale(light: &GpuLight, roughness: f32) -> f32 {
    let alpha = roughness * roughness;
    let widened = (alpha + light.gp2 / 3.0).clamp(0.0, 1.0);
    let scale = alpha / widened.max(1.0e-6);
    scale * scale
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sun(toward: Vec3, radius: f32) -> GpuLight {
        GpuLight {
            forward: toward.normalize().to_array(),
            gp1: radius.cos(),
            gp2: radius.sin(),
            ..GpuLight::default()
        }
    }

    #[test]
    fn mirror_directions_inside_the_disk_are_kept() {
        let light = sun(Vec3::Y, 0.1);
        let inside = Vec3::new(0.05, 1.0, 0.0).normalize();
        assert_eq!(specular_direction(&light, inside), inside);
    }

    #[test]
    fn mirror_directions_outside_the_disk_land_on_its_edge() {
        let radius = 0.1;
        let light = sun(Vec3::Y, radius);
        let outside = Vec3::new(1.0, 1.0, 0.3).normalize();
        let direction = specular_direction(&light, outside);
        assert!((direction.length() - 1.0).abs() < 1e-5);
        assert!(
            (direction.dot(Vec3::Y) - radius.cos()).abs() < 1e-5,
            "on the edge"
        );
        let towards = (outside - Vec3::Y * outside.y).normalize();
        assert!(
            direction.dot(towards) > 0.0,
            "on the side the mirror direction is"
        );
        assert_eq!(specular_direction(&light, Vec3::NEG_Y), Vec3::Y);
    }

    #[test]
    fn wider_disks_lower_the_peak_and_a_point_sun_keeps_it() {
        assert_eq!(specular_scale(&sun(Vec3::Y, 0.0), 0.3), 1.0);
        let small = specular_scale(&sun(Vec3::Y, 0.01), 0.3);
        let wide = specular_scale(&sun(Vec3::Y, 0.1), 0.3);
        assert!(1.0 > small && small > wide && wide > 0.0);
    }
}
