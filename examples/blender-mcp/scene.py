"""Run via execute_blender_code in a NEW Blender scene; no external assets needed.

__OUTPUT_DIR__ is substituted with a quoted absolute path by the opt-in Rust demo.
The scene is deliberately refused if an unrelated file or custom objects are open.
"""
import bpy
import math
from mathutils import Vector

output_dir = __OUTPUT_DIR__
demo_file = output_dir + "/demo.blend"
scene = bpy.context.scene
if bpy.data.filepath and bpy.data.filepath != demo_file:
    raise RuntimeError("Refusing to replace another Blender project. Open a new scene first.")
if not scene.get("hfx_blender_demo"):
    unexpected = set(bpy.data.objects.keys()) - {"Cube", "Camera", "Light"}
    if unexpected:
        raise RuntimeError("Refusing to delete custom objects: " + ", ".join(sorted(unexpected)))

bpy.ops.object.select_all(action="SELECT")
bpy.ops.object.delete(use_global=False)
scene["hfx_blender_demo"] = True
scene["demo_description"] = "A purple hfx robot holding an MCP cube, created through real MCP calls."


def material(name, color, metallic=0.0, roughness=0.4, emission=0.0):
    mat = bpy.data.materials.get(name) or bpy.data.materials.new(name)
    mat.use_nodes = True
    mat.diffuse_color = (*color, 1.0)
    node = mat.node_tree.nodes.get("Principled BSDF")
    node.inputs["Base Color"].default_value = (*color, 1.0)
    node.inputs["Metallic"].default_value = metallic
    node.inputs["Roughness"].default_value = roughness
    node.inputs["Emission Color"].default_value = (*color, 1.0)
    node.inputs["Emission Strength"].default_value = emission
    return mat


purple = material("Robot · violet", (0.28, 0.11, 0.68), 0.15, 0.28)
lilac = material("Robot · lilac trim", (0.58, 0.37, 0.95), 0.2, 0.28)
chrome = material("Joints · brushed metal", (0.23, 0.27, 0.38), 0.7, 0.3)
visor = material("Visor · obsidian", (0.012, 0.022, 0.038), 0.25, 0.22)
mint = material("Eyes · mint light", (0.15, 0.95, 0.68), 0.1, 0.3, 2.0)
white = material("Lettering · porcelain", (0.88, 0.91, 1.0), 0.1, 0.45)
gold = material("MCP cube · golden orange", (1.0, 0.37, 0.07), 0.2, 0.3)
base_mat = material("Stage · graphite", (0.035, 0.052, 0.09), 0.45, 0.32)
floor_mat = material("Backdrop · midnight blue", (0.022, 0.032, 0.062), 0.05, 0.5)
ring_mat = material("Stage · violet light", (0.41, 0.17, 0.95), 0.15, 0.3, 1.2)


def finish(obj, name, mat, smooth=True):
    obj.name = name
    obj.data.materials.append(mat)
    if smooth and obj.type == "MESH":
        for face in obj.data.polygons:
            face.use_smooth = True
    return obj


def box(name, location, dimensions, mat, bevel=0.1):
    bpy.ops.mesh.primitive_cube_add(size=1, location=location)
    obj = bpy.context.object
    obj.dimensions = dimensions
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    finish(obj, name, mat)
    modifier = obj.modifiers.new("Soft machined corners", "BEVEL")
    modifier.width = bevel
    modifier.segments = 5
    obj.modifiers.new("Weighted normals", "WEIGHTED_NORMAL")
    return obj


def sphere(name, location, scale, mat):
    bpy.ops.mesh.primitive_uv_sphere_add(segments=40, ring_count=24, radius=1, location=location)
    obj = bpy.context.object
    obj.scale = scale
    return finish(obj, name, mat)


def cylinder(name, location, radius, depth, mat, rotation=(0, 0, 0)):
    bpy.ops.mesh.primitive_cylinder_add(vertices=64, radius=radius, depth=depth, location=location, rotation=rotation)
    obj = finish(bpy.context.object, name, mat)
    modifier = obj.modifiers.new("Edge highlight", "BEVEL")
    modifier.width = 0.035
    modifier.segments = 3
    obj.modifiers.new("Weighted normals", "WEIGHTED_NORMAL")
    return obj


def limb(name, start, end, radius, mat):
    start, end = Vector(start), Vector(end)
    direction = end - start
    obj = cylinder(name, (start + end) / 2, radius, direction.length, mat)
    obj.rotation_euler = direction.to_track_quat("Z", "Y").to_euler()
    return obj


def lettering(name, text, location, size, mat, rotation=(math.pi / 2, 0, 0)):
    curve = bpy.data.curves.new(name, "FONT")
    curve.body = text
    curve.align_x = "CENTER"
    curve.align_y = "CENTER"
    curve.size = size
    curve.extrude = 0.002
    curve.bevel_depth = 0.0008
    obj = bpy.data.objects.new(name, curve)
    bpy.context.collection.objects.link(obj)
    obj.location = location
    obj.rotation_euler = rotation
    curve.materials.append(mat)
    return obj


# Small illuminated presentation stage.
cylinder("Presentation plinth", (0, 0, 0.15), 1.95, 0.3, base_mat)
bpy.ops.mesh.primitive_torus_add(major_radius=1.84, minor_radius=0.025, major_segments=128, minor_segments=16, location=(0, 0, 0.29))
finish(bpy.context.object, "Violet perimeter light", ring_mat)
lettering("Stage inscription", "hfx  /  BLENDER MCP", (0, -1.38, 0.306), 0.17, white, rotation=(0, 0, 0))

# Boots, legs, torso, and display badge.
for side, x in (("Left", -0.43), ("Right", 0.43)):
    box(side + " boot", (x, -0.11, 0.53), (0.55, 0.78, 0.4), lilac, 0.13)
    cylinder(side + " shin", (x, 0, 0.96), 0.15, 0.47, chrome)
    sphere(side + " hip", (x, 0, 1.21), (0.2, 0.2, 0.2), chrome)
box("Robot body", (0, 0, 1.77), (1.13, 0.78, 1.16), purple, 0.18)
box("hfx badge bezel", (0, -0.40, 2.03), (0.66, 0.12, 0.26), visor, 0.055)
lettering("hfx badge", "hfx", (0, -0.465, 2.03), 0.22, white)
for index, x in enumerate((-0.15, 0, 0.15)):
    sphere("Status light " + str(index + 1), (x, -0.406, 1.63), (0.037, 0.027, 0.037), mint)
cylinder("Neck joint", (0, 0, 2.45), 0.2, 0.22, chrome)

# Rounded head and illuminated face.
box("Robot head", (0, 0, 3.05), (1.74, 1.08, 1.12), purple, 0.22)
box("Face visor", (0, -0.555, 3.04), (1.43, 0.32, 0.68), visor, 0.145)
for side, x in (("Left", -0.33), ("Right", 0.33)):
    sphere(side + " eye", (x, -0.724, 3.13), (0.115, 0.035, 0.145), mint)
smile = bpy.data.curves.new("Smile curve", "CURVE")
smile.dimensions = "3D"
smile.bevel_depth = 0.023
smile.bevel_resolution = 5
spline = smile.splines.new("POLY")
spline.points.add(16)
for index, point in enumerate(spline.points):
    t = index / 8 - 1
    point.co = (0.17 * t, -0.725, 2.85 + 0.07 * t * t, 1)
smile.materials.append(mint)
obj = bpy.data.objects.new("Mint smile", smile)
bpy.context.collection.objects.link(obj)
for side, x in (("Left", -0.96), ("Right", 0.96)):
    cylinder(side + " ear", (x, 0, 3.05), 0.27, 0.22, lilac, (0, math.pi / 2, 0))
    cylinder(side + " ear cap", (x * 1.13, 0, 3.05), 0.16, 0.04, chrome, (0, math.pi / 2, 0))
cylinder("Antenna", (0, 0.05, 3.76), 0.045, 0.31, chrome)
sphere("Antenna signal", (0, 0.05, 3.96), (0.13, 0.13, 0.13), mint)

# One relaxed arm; the other proudly holds an MCP cube.
for name, start, elbow, hand in (
    ("Left", (-0.72, 0, 2.14), (-1.03, -0.1, 1.77), (-1.10, -0.26, 1.36)),
    ("Right", (0.72, 0, 2.14), (1.08, -0.08, 2.08), (1.30, -0.20, 2.46)),
):
    sphere(name + " shoulder", start, (0.22, 0.22, 0.22), chrome)
    limb(name + " upper arm", start, elbow, 0.13, lilac)
    sphere(name + " elbow", elbow, (0.15, 0.15, 0.15), chrome)
    limb(name + " forearm", elbow, hand, 0.135, purple)
    sphere(name + " hand", hand, (0.19, 0.19, 0.19), lilac)
box("MCP cube", (1.43, -0.22, 2.91), (0.67, 0.67, 0.67), gold, 0.075)
lettering("MCP cube label", "MCP", (1.43, -0.559, 2.91), 0.18, white)

# Seamless studio, camera, and three large softboxes.
bpy.ops.mesh.primitive_plane_add(size=200, location=(0, 0, -0.015))
finish(bpy.context.object, "Studio backdrop", floor_mat, smooth=False)
world = bpy.data.worlds.new("Midnight studio world")
scene.world = world
world.use_nodes = True
world.node_tree.nodes["Background"].inputs["Color"].default_value = (0.045, 0.06, 0.1, 1)
world.node_tree.nodes["Background"].inputs["Strength"].default_value = 0.35


def aim(obj, target):
    obj.rotation_euler = (Vector(target) - obj.location).to_track_quat("-Z", "Y").to_euler()


for name, location, energy, color, size in (
    ("Soft key", (-4, -5, 7), 1100, (0.90, 0.93, 1.0), 5),
    ("Mint fill", (4, -3, 5), 850, (0.57, 1.0, 0.88), 4),
    ("Violet rim", (2, 4, 7), 1400, (0.71, 0.55, 1.0), 3),
):
    bpy.ops.object.light_add(type="AREA", location=location)
    light = bpy.context.object
    light.name = name
    light.data.energy = energy
    light.data.color = color
    light.data.shape = "DISK"
    light.data.size = size
    aim(light, (0, 0, 1.9))
bpy.ops.object.camera_add(location=(5.5, -10, 5.7))
camera = bpy.context.object
camera.name = "Demo camera"
camera.data.type = "ORTHO"
camera.data.ortho_scale = 6.35
aim(camera, (0.08, 0, 2.05))
scene.camera = camera
scene.render.engine = "CYCLES"
scene.cycles.samples = 40
scene.cycles.use_denoising = True
scene.render.resolution_x = 1280
scene.render.resolution_y = 1000
scene.render.resolution_percentage = 100
scene.render.image_settings.file_format = "PNG"
scene.render.filepath = output_dir + "/render.png"
scene.view_settings.view_transform = "AgX"
scene.render.film_transparent = False

# Present a clean camera viewport for the MCP screenshot tool.
bpy.ops.object.select_all(action="DESELECT")
for screen in bpy.data.screens:
    for area in screen.areas:
        if area.type == "VIEW_3D":
            space = area.spaces.active
            space.overlay.show_overlays = False
            space.shading.type = "SOLID"
            space.shading.color_type = "MATERIAL"
            space.shading.light = "STUDIO"
            space.shading.show_shadows = True
            space.shading.show_cavity = True
            space.region_3d.view_perspective = "CAMERA"
            space.region_3d.view_camera_zoom = 5

bpy.ops.wm.save_as_mainfile(filepath=demo_file)
print("HFX_DEMO_CREATED", len(scene.objects), "objects", demo_file)
