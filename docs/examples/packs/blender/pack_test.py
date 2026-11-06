"""The Blender pack's test: add a cube, give it a red material, render one frame to a file.

Run: blender --background --factory-startup --python-exit-code 1 --python pack_test.py -- OUT.png
Exit code 0 and a non-empty PNG at OUT.png mean it passed.
"""
import sys

import bpy

out = sys.argv[sys.argv.index("--") + 1]

bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene

bpy.ops.mesh.primitive_cube_add(size=2, location=(0, 0, 0))
cube = bpy.context.active_object

mat = bpy.data.materials.new("pack-test-red")
if mat.node_tree is None:  # an older Blender has no node tree until use_nodes is set
    mat.use_nodes = True
bsdf = mat.node_tree.nodes.get("Principled BSDF")
bsdf.inputs["Base Color"].default_value = (0.8, 0.05, 0.05, 1.0)
cube.data.materials.append(mat)

bpy.ops.object.light_add(type="SUN", location=(3, -3, 5))
cam_data = bpy.data.cameras.new("cam")
cam = bpy.data.objects.new("cam", cam_data)
scene.collection.objects.link(cam)
cam.location = (5, -5, 4)
cam.rotation_euler = (1.0, 0.0, 0.785)
scene.camera = cam

scene.render.engine = "CYCLES"
scene.cycles.device = "CPU"
scene.cycles.samples = 4
scene.render.resolution_x = 160
scene.render.resolution_y = 120
scene.render.image_settings.file_format = "PNG"
scene.render.filepath = out

bpy.ops.render.render(write_still=True)
print("pack-test rendered", out)
