"""Builds tests/fixtures/rooms.gltf (and rooms.bin) for tests/gltf_rooms.rs.

Run from anywhere; the files are written next to this script:

    blender --background --factory-startup --python tests/fixtures/rooms_gltf.py

Everything sits under "Level", an empty at (10, 0, 0) turned 90 degrees about Z, so the rooms
and portals are checked under a parent's transform. In Blender coordinates (Bevy has
x, z, -y):

- "Room A": a Cube empty sized only by its scale, with dynlight_room = True.
- "Room B": a mesh with unit-cube geometry and two materials (two glTF primitives), with
  dynlight_room = 1 and a real child mesh node, "Crate", which must survive.
- "Door": a Cube empty between the rooms, with dynlight_portal = True. Width runs along its
  local X, height along Z and thickness along Y.
- "Window": a Cube empty on Room B's far wall, with dynlight_portal and
  dynlight_portal_closed.
- "Lamp": a point light empty in Room A. "Floor": a plane mesh with dynlight_static.
"""

import math
import os

import bmesh
import bpy

HERE = os.path.dirname(os.path.abspath(__file__))

bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene


def place(obj, parent, location, rotation_z=0.0, scale=(1.0, 1.0, 1.0)):
    scene.collection.objects.link(obj)
    obj.parent = parent
    obj.location = location
    obj.rotation_euler = (0.0, 0.0, math.radians(rotation_z))
    obj.scale = scale
    return obj


def empty(name, parent, location, rotation_z=0.0, scale=(1.0, 1.0, 1.0), cube=True):
    obj = bpy.data.objects.new(name, None)
    obj.empty_display_type = "CUBE" if cube else "PLAIN_AXES"
    obj.empty_display_size = 1.0
    return place(obj, parent, location, rotation_z, scale)


def material(name, color):
    mat = bpy.data.materials.new(name)
    mat.diffuse_color = color
    return mat


def mesh(name, build, materials):
    data = bpy.data.meshes.new(name)
    bm = bmesh.new()
    build(bm)
    bm.to_mesh(data)
    bm.free()
    for mat in materials:
        data.materials.append(mat)
    return bpy.data.objects.new(name, data)


def unit_cube(bm):
    bmesh.ops.create_cube(bm, size=2.0)
    for face in bm.faces:
        face.material_index = 1 if face.normal.z > 0.5 else 0


wall = material("Wall", (0.8, 0.8, 0.8, 1.0))
trim = material("Trim", (0.6, 0.3, 0.1, 1.0))

level = empty("Level", None, (10.0, 0.0, 0.0), rotation_z=90.0, cube=False)

room_a = empty("Room A", level, (0.0, 0.0, 1.5), scale=(3.0, 2.0, 1.5))
room_a["dynlight_room"] = True

room_b = place(mesh("Room B", unit_cube, [wall, trim]), level, (5.0, 0.0, 1.5), scale=(2.0, 2.0, 1.5))
room_b["dynlight_room"] = 1

crate = mesh("Crate", lambda bm: bmesh.ops.create_cube(bm, size=0.4), [trim])
place(crate, room_b, (0.5, 0.5, -0.8))

door = empty("Door", level, (3.0, 0.0, 1.1), rotation_z=90.0, scale=(0.6, 0.05, 1.1))
door["dynlight_portal"] = True

window = empty("Window", level, (7.0, 0.0, 1.5), rotation_z=90.0, scale=(1.0, 0.05, 0.6))
window["dynlight_portal"] = True
window["dynlight_portal_closed"] = True

lamp = empty("Lamp", level, (0.0, 0.0, 2.5), cube=False)
lamp["dynlight_type"] = "point"
lamp["dynlight_radius"] = 3.0

floor = mesh("Floor", lambda bm: bmesh.ops.create_grid(bm, x_segments=1, y_segments=1, size=1.0), [wall])
place(floor, level, (2.0, 0.0, 0.0))
floor["dynlight_static"] = True

bpy.ops.export_scene.gltf(
    filepath=os.path.join(HERE, "rooms.gltf"),
    export_format="GLTF_SEPARATE",
    export_extras=True,
    export_yup=True,
    export_apply=False,
    export_cameras=False,
    export_lights=False,
    export_animations=False,
)
