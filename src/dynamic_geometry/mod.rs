//! Lighting for meshes without baked data (moving or unbaked meshes): light BVH traversal in the
//! shader and baked distance cubes, in the angular or distance-cube mode. The data comes from
//! the bake through the baked-lighting aux buffer.
//!
//! Shader modules: `dynlight::dynamic_geometry::shaders::{light_bvh, distance_cubes}`.

#[cfg(test)]
pub(crate) mod shader_tests;

use bevy::prelude::*;
use bevy::shader::load_shader_library;

use crate::shared::shaders_available;

pub struct DynamicGeometryPlugin;

impl Plugin for DynamicGeometryPlugin {
    fn build(&self, _app: &mut App) {}

    fn finish(&self, app: &mut App) {
        if shaders_available(app) {
            load_shader_library!(app, "shaders/light_bvh.wesl");
            load_shader_library!(app, "shaders/distance_cubes.wesl");
        }
    }
}
