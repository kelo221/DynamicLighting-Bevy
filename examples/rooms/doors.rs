//! Door leaves: every door and the window has one, hidden while it is open. `C` shuts or opens
//! them all (`--closed-doors` starts with them shut). A shut door shows its leaf, which casts
//! shadows, and marks its portal [`ClosedPortal`], so what lies only past it drops out of view.

use bevy::prelude::*;
use dynlight::prelude::*;

/// The leaf that shuts the door portal it names.
#[derive(Component)]
pub struct Leaf(pub Entity);

/// Shuts or opens the door `portal`.
pub fn shut(commands: &mut Commands, portal: Entity, shut: bool) {
    if shut {
        commands.entity(portal).insert(ClosedPortal);
    } else {
        commands.entity(portal).remove::<ClosedPortal>();
    }
}

/// `--closed-doors`: every door starts shut.
pub fn shut_at_start(mut commands: Commands, portals: Query<Entity, With<Portal>>) {
    for portal in &portals {
        shut(&mut commands, portal, true);
    }
}

/// `C`: shuts every door while any is open, and opens them all once they are shut.
pub fn toggle(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    portals: Query<(Entity, Has<ClosedPortal>), With<Portal>>,
) {
    if !keys.just_pressed(KeyCode::KeyC) {
        return;
    }
    let any_open = portals.iter().any(|(_, closed)| !closed);
    for (portal, _) in &portals {
        shut(&mut commands, portal, any_open);
    }
}

/// Shows the leaves of shut doors and hides the others.
pub fn show_leaves(
    mut leaves: Query<(&Leaf, &mut Visibility)>,
    portals: Query<Has<ClosedPortal>, With<Portal>>,
) {
    for (leaf, mut visibility) in &mut leaves {
        let shown = portals.get(leaf.0).unwrap_or(false);
        visibility.set_if_neq(if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
}
