//! `Name` paths through the entity hierarchy: stable across runs, unlike entity ids, which shift
//! whenever anything is spawned earlier.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

/// Looks up `Name` paths through the entity hierarchy.
#[derive(SystemParam)]
pub struct NamePaths<'w, 's> {
    hierarchy: Query<'w, 's, (Option<&'static Name>, Option<&'static ChildOf>)>,
}

impl NamePaths<'_, '_> {
    /// `Name`s from the root down to `entity`; unnamed entities count as empty segments.
    pub fn name_path(&self, entity: Entity) -> Vec<String> {
        let mut path = Vec::new();
        let mut current = Some(entity);
        while let Some(entity) = current.take() {
            let Ok((name, parent)) = self.hierarchy.get(entity) else {
                break;
            };
            path.push(name.map_or_else(String::new, |name| name.as_str().to_owned()));
            // Guards against malformed hierarchies; real ones are far shallower.
            if path.len() < 1024 {
                current = parent.map(ChildOf::parent);
            }
        }
        path.reverse();
        path
    }
}
