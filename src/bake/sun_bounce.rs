//! Sun bounce: per texel, rays over the hemisphere around the face normal with a cosine density.
//! A ray whose first hit is the front of a surface that faces and sees the sun brings back that
//! surface's sunlit share, `cos θ_sun` (times its color, see [`reflected`]). With the cosine
//! density the average over all rays is the bounce irradiance in the units of the sun's direct
//! light, which the shader scales by the sun's color and intensity like a local light's bounce.
//!
//! Henry's bounce comes from photon cubes around a light's position; a sun has none, and its
//! light reaches far more surfaces than a cube could resolve, so it gathers from the texel.

use bevy::math::{Vec2, Vec3};

use super::bounce::{Gathered, reflected};
use super::input::BakeLight;
use super::mesh_bake::{RAY_OFFSET, Scene, TriangleSetup};
use super::mesh_builder::PreparedMesh;
use super::occluder::RayCaster;
use super::random;
use super::sky::cosine_direction;
use super::spread::spread;
use super::texels::SurfaceTexel;

/// One triangle's sun bounce: `light.bounce.samples` rays per texel through the Sobol (0, 2)
/// points, shifted per texel, then `sum · intensity / samples`.
pub fn gather_sun(
    scene: &Scene,
    texels: &[SurfaceTexel],
    setup: &TriangleSetup,
    prepared: &PreparedMesh,
    light: &BakeLight,
    toward: Vec3,
) -> Gathered {
    if setup.face_normal == Vec3::ZERO {
        return (Vec::new(), 0);
    }
    let samples = light.bounce.samples.max(2);
    let alpha_test = light.alpha_tested();
    let normal = setup.face_normal;
    let occluders = &scene.occluders;
    let mut rays = 0u64;
    let mut out = Vec::with_capacity(texels.len());
    for texel in texels {
        let origin = texel.position + normal * RAY_OFFSET;
        let mut rng = random::stream(&[
            scene.options.seed,
            prepared.mesh.id.0,
            light.id.0,
            u64::from(texel.x),
            u64::from(texel.y),
        ]);
        let offset = Vec2::new(rng.next_f32(), rng.next_f32());
        let mut sum = Vec3::ZERO;
        for i in 0..samples {
            let direction = cosine_direction(normal, spread(i, offset, true));
            rays += 1;
            let Some(hit) = occluders.closest_hit(origin, direction, f32::INFINITY, alpha_test)
            else {
                continue;
            };
            let face = occluders.face_normal(&hit);
            if face.dot(direction) >= 0.0 {
                continue;
            }
            let lit = occluders.surface_normal(&hit).dot(toward);
            if lit <= 0.0 || face.dot(toward) <= 0.0 {
                continue;
            }
            let point = origin + direction * hit.distance + face * RAY_OFFSET;
            rays += 1;
            if !occluders.occluded(point, toward, f32::INFINITY, alpha_test) {
                sum += reflected(scene, occluders.albedo(&hit)) * lit;
            }
        }
        out.push((
            texel.x,
            texel.y,
            sum * light.bounce.intensity / samples as f32,
        ));
    }
    (out, rays)
}
