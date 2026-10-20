//! The six rooms off a corridor with a window onto a yard, and the parts the grid of rooms
//! shares. Room cells run to the middle of the walls, and every door and window is a portal.

use bevy::prelude::*;
use dynlight::prelude::*;

use crate::doors::Leaf;

pub const HEIGHT: f32 = 3.0;
pub const WALL: f32 = 0.2;
/// Door leaves are this thick, in the middle of the wall.
const LEAF: f32 = 0.04;
pub const DOOR: Vec2 = Vec2::new(1.2, 2.2);
/// Rooms are this wide along the corridor, and the grid's rooms this wide both ways.
pub const ROOM: f32 = 8.0;
const CORRIDOR: f32 = 3.0;
/// How far each north and south door sits from the middle of its room.
pub const NORTH_DOORS: [f32; 3] = [-2.0, 1.5, -1.0];
pub const SOUTH_DOORS: [f32; 3] = [2.0, -1.5, 0.5];

/// A light that bobs up and down unless `--still`.
#[derive(Component)]
pub struct Bob {
    pub anchor: Vec3,
    pub phase: f32,
}

/// Lights and shadow cubemaps in the scene: `--grid n` has three lights and one cube per room.
pub fn counts(grid: Option<u32>) -> (u32, u32) {
    match grid {
        Some(n) => (3 * n * n, n * n),
        None => (21, 7),
    }
}

/// Where the camera looks from and at by default.
pub fn view(grid: Option<u32>) -> Transform {
    match grid {
        Some(n) => {
            let far = ROOM * n as f32 - 1.0;
            Transform::from_xyz(1.0, 1.7, 1.0).looking_at(Vec3::new(far, 1.2, far), Vec3::Y)
        }
        None => Transform::from_xyz(1.0, 1.7, -0.8).looking_at(Vec3::new(14.0, 1.2, 1.4), Vec3::Y),
    }
}

/// Meshes and materials shared by every room.
pub struct Kit {
    cube: Handle<Mesh>,
    plaster: Handle<DynLitMaterial>,
    floor: Handle<DynLitMaterial>,
    wood: Handle<DynLitMaterial>,
}

impl Kit {
    pub fn new(meshes: &mut Assets<Mesh>, materials: &mut Assets<DynLitMaterial>) -> Self {
        Self {
            cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
            plaster: materials.add(Color::srgb(0.75, 0.74, 0.72)),
            floor: materials.add(Color::srgb(0.55, 0.5, 0.46)),
            wood: materials.add(Color::srgb(0.8, 0.58, 0.36)),
        }
    }

    /// A box from `min` to `max`.
    pub fn block(
        &self,
        commands: &mut Commands,
        material: &Handle<DynLitMaterial>,
        min: Vec3,
        max: Vec3,
    ) {
        let size = max - min;
        if size.min_element() <= 1.0e-3 {
            return;
        }
        commands.spawn((
            Mesh3d(self.cube.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_translation((min + max) * 0.5).with_scale(size),
        ));
    }

    /// A wall whose middle is the plane `x = at` (`along_z`) or `z = at`, from `start` to `end`
    /// along it, with doors centered at `doors` and `windows` as (from, to, bottom, top).
    pub fn wall(
        &self,
        commands: &mut Commands,
        along_z: bool,
        at: f32,
        (start, end): (f32, f32),
        doors: &[f32],
        windows: &[(f32, f32, f32, f32)],
    ) {
        let mut openings: Vec<(f32, f32, f32, f32)> = doors
            .iter()
            .map(|&center| (center - DOOR.x / 2.0, center + DOOR.x / 2.0, 0.0, DOOR.y))
            .chain(windows.iter().copied())
            .collect();
        openings.sort_by(|a, b| a.0.total_cmp(&b.0));
        // Walls along X close the corners; walls along Z stop short of them.
        let (start, end) = if along_z {
            (start + WALL / 2.0, end - WALL / 2.0)
        } else {
            (start - WALL / 2.0, end + WALL / 2.0)
        };
        let mut span = |from: f32, to: f32, bottom: f32, top: f32| {
            let (min, max) = if along_z {
                (
                    Vec3::new(at - WALL / 2.0, bottom, from),
                    Vec3::new(at + WALL / 2.0, top, to),
                )
            } else {
                (
                    Vec3::new(from, bottom, at - WALL / 2.0),
                    Vec3::new(to, top, at + WALL / 2.0),
                )
            };
            self.block(commands, &self.plaster, min, max);
        };
        let mut cursor = start;
        for (from, to, bottom, top) in openings {
            span(cursor, from, 0.0, HEIGHT);
            span(from, to, 0.0, bottom);
            span(from, to, top, HEIGHT);
            cursor = to;
        }
        span(cursor, end, 0.0, HEIGHT);
    }

    /// The floor and ceiling over `min..max` (X and Z).
    pub fn slabs(&self, commands: &mut Commands, min: Vec2, max: Vec2) {
        let (low, high) = (min - WALL / 2.0, max + WALL / 2.0);
        let floor = (
            Vec3::new(low.x, -0.1, low.y),
            Vec3::new(high.x, 0.0, high.y),
        );
        self.block(commands, &self.floor, floor.0, floor.1);
        let ceiling = (
            Vec3::new(low.x, HEIGHT, low.y),
            Vec3::new(high.x, HEIGHT + 0.1, high.y),
        );
        self.block(commands, &self.plaster, ceiling.0, ceiling.1);
    }

    /// A crate and a pillar for the shadow light to cast shadows of.
    pub fn props(&self, commands: &mut Commands, center: Vec3, side: f32) {
        let center = center.with_y(0.0);
        let crate_at = center + Vec3::new(1.2, 0.0, -0.6 * side);
        self.block(
            commands,
            &self.wood,
            crate_at + Vec3::new(-0.45, 0.0, -0.45),
            crate_at + Vec3::new(0.45, 0.9, 0.45),
        );
        let pillar = center + Vec3::new(-1.6, 0.0, 1.0 * side);
        self.block(
            commands,
            &self.plaster,
            pillar + Vec3::new(-0.2, 0.0, -0.2),
            pillar + Vec3::new(0.2, HEIGHT, 0.2),
        );
    }
}

pub fn room(commands: &mut Commands, name: String, center: Vec3, size: Vec3) {
    commands.spawn((
        Name::new(name),
        Room::new(size),
        Transform::from_translation(center),
    ));
}

/// A door portal in a wall facing `facing` (±X or ±Z), with a leaf that shows while it is shut.
pub fn door(commands: &mut Commands, kit: &Kit, name: String, at: Vec3, size: Vec2, facing: Vec3) {
    let transform = Transform::from_translation(at).looking_to(-facing, Vec3::Y);
    let portal = commands
        .spawn((Name::new(name.clone()), Portal::new(size), transform))
        .id();
    // A centimeter into the wall all round, so a shut door seals the opening.
    let leaf = (size + Vec2::splat(0.02)).extend(LEAF);
    commands.spawn((
        Name::new(format!("{name} leaf")),
        Leaf(portal),
        Mesh3d(kit.cube.clone()),
        MeshMaterial3d(kit.wood.clone()),
        transform.with_scale(leaf),
        Visibility::Hidden,
    ));
}

fn point(color: Color, radius: f32, intensity: f32) -> DynamicLight {
    DynamicLight {
        color,
        radius,
        intensity,
        realtime: true,
        ..DynamicLight::point()
    }
}

/// A room's three lights: a shadowed one in the middle, one at the back (a spot in a cone of
/// fog, with `fog`) and one by the door.
pub fn room_lights(
    commands: &mut Commands,
    name: &str,
    (center, side): (Vec3, f32),
    door_x: f32,
    hue: f32,
    fog: bool,
) {
    let hue = |shift: f32| (hue + shift).rem_euclid(360.0);
    // Big enough to light the room, so it also reaches the rooms beside it.
    let shadowed = DynamicLight {
        shadows: ShadowMode::Realtime,
        ..point(Color::hsl(hue(0.0), 0.35, 0.75), 5.5, 2.0)
    };
    let anchor = center + Vec3::new(0.0, 0.9, 0.3 * side);
    commands.spawn((
        Name::new(format!("{name} lamp")),
        shadowed,
        Transform::from_translation(anchor),
        Bob {
            anchor,
            phase: hue(0.0).to_radians(),
        },
    ));
    let back = point(Color::hsl(hue(40.0), 0.7, 0.6), 3.0, 1.2);
    if fog {
        commands.spawn((
            Name::new(format!("{name} fog spot")),
            DynamicLight {
                light_type: LightType::Spot,
                radius: 3.5,
                inner_cutoff: 25.0,
                outer_cutoff: 35.0,
                ..back
            },
            VolumetricLight {
                shape: VolumetricShape::ConeZ,
                radius: 2.6,
                ..default()
            },
            Transform::from_translation(center + Vec3::new(1.8, 1.4, 2.2 * side))
                .looking_to(Vec3::NEG_Y, Vec3::Z),
        ));
    } else {
        commands.spawn((
            Name::new(format!("{name} back light")),
            back,
            Transform::from_translation(center + Vec3::new(2.4, -0.6, 2.9 * side)),
        ));
    }
    commands.spawn((
        Name::new(format!("{name} door light")),
        point(Color::hsl(hue(180.0), 0.6, 0.65), 2.5, 1.0),
        Transform::from_xyz(door_x, 2.6, center.z - 2.9 * side),
    ));
}

/// Six rooms off a corridor along +X; the last north room has a window onto a yard.
pub fn corridor(commands: &mut Commands, kit: &Kit) {
    let depth = ROOM;
    let edge = CORRIDOR / 2.0;
    let far = edge + depth;
    let length = ROOM * 3.0;
    kit.slabs(commands, Vec2::new(0.0, -far), Vec2::new(length, far));
    room(
        commands,
        "Corridor".into(),
        Vec3::new(length / 2.0, HEIGHT / 2.0, 0.0),
        Vec3::new(length, HEIGHT, CORRIDOR),
    );
    commands.spawn((
        Name::new("Corridor west light"),
        point(Color::srgb(1.0, 0.92, 0.8), 5.0, 1.0),
        Transform::from_xyz(6.0, 2.6, 0.0),
    ));
    commands.spawn((
        Name::new("Corridor east light"),
        point(Color::srgb(0.85, 0.9, 1.0), 5.0, 1.0),
        Transform::from_xyz(18.0, 2.6, 0.0),
    ));

    let window = (19.0, 21.0, 1.0, 2.2);
    for (side, offsets, label) in [(1.0, NORTH_DOORS, "North"), (-1.0, SOUTH_DOORS, "South")] {
        let doors: Vec<f32> = (0..3)
            .map(|i| ROOM * i as f32 + ROOM / 2.0 + offsets[i])
            .collect();
        kit.wall(commands, false, edge * side, (0.0, length), &doors, &[]);
        let windows: &[_] = if side > 0.0 { &[window] } else { &[] };
        kit.wall(commands, false, far * side, (0.0, length), &[], windows);
        let span = if side > 0.0 {
            (edge, far)
        } else {
            (-far, -edge)
        };
        for x in [ROOM, ROOM * 2.0] {
            kit.wall(commands, true, x, span, &[], &[]);
        }
        for (i, &door_x) in doors.iter().enumerate() {
            let name = format!("{label} room {i}");
            let center = Vec3::new(
                ROOM * i as f32 + ROOM / 2.0,
                HEIGHT / 2.0,
                (edge + depth / 2.0) * side,
            );
            room(
                commands,
                name.clone(),
                center,
                Vec3::new(ROOM, HEIGHT, depth),
            );
            door(
                commands,
                kit,
                format!("{name} door"),
                Vec3::new(door_x, DOOR.y / 2.0, edge * side),
                DOOR,
                Vec3::Z * side,
            );
            let hue = 50.0 * i as f32 + 150.0 * (side + 1.0);
            let fog = side > 0.0 && i == 1;
            room_lights(commands, &name, (center, side), door_x, hue, fog);
            kit.props(commands, center, side);
        }
    }
    for x in [0.0, length] {
        kit.wall(commands, true, x, (-far, far), &[], &[]);
    }

    // The yard beyond the window, outside every room.
    let (from, to, bottom, top) = window;
    door(
        commands,
        kit,
        "Yard window".into(),
        Vec3::new((from + to) / 2.0, (bottom + top) / 2.0, far),
        Vec2::new(to - from, top - bottom),
        Vec3::Z,
    );
    let ground = (
        Vec3::new(8.0, -0.1, far + WALL / 2.0),
        Vec3::new(32.0, 0.0, far + 14.0),
    );
    kit.block(commands, &kit.floor, ground.0, ground.1);
    kit.block(
        commands,
        &kit.plaster,
        Vec3::new(21.2, 0.0, far + 2.3),
        Vec3::new(21.7, HEIGHT, far + 2.8),
    );
    let anchor = Vec3::new(20.0, 2.6, far + 4.0);
    commands.spawn((
        Name::new("Yard light"),
        DynamicLight {
            shadows: ShadowMode::Realtime,
            ..point(Color::srgb(0.7, 0.8, 1.0), 8.0, 1.4)
        },
        Transform::from_translation(anchor),
        Bob { anchor, phase: 2.0 },
    ));
}
