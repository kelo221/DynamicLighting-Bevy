//! `--grid n`: `n × n` rooms of 8 m, each joined to its neighbors by a door at a stable,
//! pseudo-random place along the wall.

use bevy::prelude::*;

use crate::layout::{DOOR, HEIGHT, Kit, ROOM, door, room, room_lights};

/// A stable pseudo-random offset in `-2.5..2.5` for door `index`.
fn offset(index: u32) -> f32 {
    let mut x = index.wrapping_mul(0x9e37_79b9) ^ 0x85eb_ca6b;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2c1b_3c6d);
    x ^= x >> 12;
    (x & 0xffff) as f32 / 65_535.0 * 5.0 - 2.5
}

/// The door between rooms `i - 1` and `i` along X in row `j` (`east`), or between rows `j - 1`
/// and `j` in column `i`: its offset from the middle of the room.
pub fn grid_door(i: u32, j: u32, east: bool) -> f32 {
    offset(i * 4099 + j * 8 + u32::from(east))
}

/// `n × n` rooms of 8 m, each joined to its neighbors by a door.
pub fn spawn(commands: &mut Commands, kit: &Kit, n: u32) {
    let size = ROOM * n as f32;
    kit.slabs(commands, Vec2::ZERO, Vec2::splat(size));
    for line in 0..=n {
        let at = ROOM * line as f32;
        let inner = line > 0 && line < n;
        let doors = |east: bool| -> Vec<f32> {
            if !inner {
                return Vec::new();
            }
            (0..n)
                .map(|k| {
                    let (i, j) = if east { (line, k) } else { (k, line) };
                    ROOM * k as f32 + ROOM / 2.0 + grid_door(i, j, east)
                })
                .collect()
        };
        kit.wall(commands, true, at, (0.0, size), &doors(true), &[]);
        kit.wall(commands, false, at, (0.0, size), &doors(false), &[]);
        if !inner {
            continue;
        }
        for k in 0..n {
            let middle = ROOM * k as f32 + ROOM / 2.0;
            let east = Vec3::new(at, DOOR.y / 2.0, middle + grid_door(line, k, true));
            door(
                commands,
                kit,
                format!("Door {line},{k} east"),
                east,
                DOOR,
                Vec3::X,
            );
            let north = Vec3::new(middle + grid_door(k, line, false), DOOR.y / 2.0, at);
            door(
                commands,
                kit,
                format!("Door {k},{line} north"),
                north,
                DOOR,
                Vec3::Z,
            );
        }
    }
    for i in 0..n {
        for j in 0..n {
            let name = format!("Room {i},{j}");
            let center = Vec3::new(
                ROOM * (i as f32 + 0.5),
                HEIGHT / 2.0,
                ROOM * (j as f32 + 0.5),
            );
            room(
                commands,
                name.clone(),
                center,
                Vec3::new(ROOM, HEIGHT, ROOM),
            );
            let door_x = center.x + grid_door(i, j, false);
            let hue = ((i * 7 + j * 3) % 12) as f32 * 30.0;
            room_lights(commands, &name, (center, 1.0), door_x, hue, false);
            kit.props(commands, center, 1.0);
        }
    }
}
