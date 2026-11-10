"""Build the photo-referenced Dockstream 2 exterior with Blender's Python.

Run: blender -b --python-exit-code 1 -P assets/model/dockstream2.py
Dimensions are nominal, not measured manufacturing geometry. Reference photos
remain private; all materials here are procedural and contain no photographs.
"""

import math
from pathlib import Path

import bpy
import bmesh
from mathutils import Vector

HERE = Path(__file__).resolve().parent
TRAY_FLOOR_Z = 0.1508


def material(name, color, roughness, metal=0.0):
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    shader = next(n for n in mat.node_tree.nodes if n.type == "BSDF_PRINCIPLED")
    shader.inputs["Base Color"].default_value = (*color, 1)
    shader.inputs["Roughness"].default_value = roughness
    shader.inputs["Metallic"].default_value = metal
    mat.diffuse_color = (*color, 1)
    return mat, shader


def metric_noise(mat, label, frequency, detail=2, warp=False):
    """Object coordinates keep grain size in meters, independent of bounds."""
    nodes, links = mat.node_tree.nodes, mat.node_tree.links
    coords = nodes.new("ShaderNodeTexCoord")
    coords.label = label + " · metric coordinates"
    stretch = nodes.new("ShaderNodeVectorMath")
    stretch.operation = "MULTIPLY"
    stretch.inputs[1].default_value = frequency
    position = coords.outputs["Object"]
    if warp:
        # Slow sub-millimeter drift breaks parallel fibers without creating
        # broad marbling or the knots of an actual cut wooden board.
        drift = nodes.new("ShaderNodeTexNoise")
        drift.label = "Slow fiber direction drift"
        drift.inputs["Scale"].default_value = 35
        drift.inputs["Detail"].default_value = 2
        links.new(position, drift.inputs["Vector"])
        center = nodes.new("ShaderNodeVectorMath")
        center.operation = "SUBTRACT"
        center.inputs[1].default_value = (0.5, 0.5, 0.5)
        links.new(drift.outputs["Color"], center.inputs[0])
        amplitude = nodes.new("ShaderNodeVectorMath")
        amplitude.operation = "MULTIPLY"
        amplitude.inputs[1].default_value = (0.0007, 0.003, 0.0003)
        links.new(center.outputs["Vector"], amplitude.inputs[0])
        offset = nodes.new("ShaderNodeVectorMath")
        offset.operation = "ADD"
        links.new(position, offset.inputs[0])
        links.new(amplitude.outputs["Vector"], offset.inputs[1])
        position = offset.outputs["Vector"]
    links.new(position, stretch.inputs[0])
    noise = nodes.new("ShaderNodeTexNoise")
    noise.label = label
    noise.inputs["Scale"].default_value = 1
    noise.inputs["Detail"].default_value = detail
    noise.inputs["Roughness"].default_value = 0.65
    links.new(stretch.outputs["Vector"], noise.inputs["Vector"])
    return noise.outputs["Fac"]


def mapped_field(mat, field, low, high, target):
    remap = mat.node_tree.nodes.new("ShaderNodeMapRange")
    remap.inputs["To Min"].default_value = low
    remap.inputs["To Max"].default_value = high
    mat.node_tree.links.new(field, remap.inputs["Value"])
    mat.node_tree.links.new(remap.outputs["Result"], target)


def surface_bump(mat, field, shader, distance, strength):
    bump = mat.node_tree.nodes.new("ShaderNodeBump")
    bump.inputs["Strength"].default_value = strength
    bump.inputs["Distance"].default_value = distance
    mat.node_tree.links.new(field, bump.inputs["Height"])
    mat.node_tree.links.new(bump.outputs["Normal"], shader.inputs["Normal"])


def blend_fields(mat, first, second, weight):
    blend = mat.node_tree.nodes.new("ShaderNodeMixRGB")
    blend.inputs[0].default_value = weight
    mat.node_tree.links.new(first, blend.inputs[1])
    mat.node_tree.links.new(second, blend.inputs[2])
    return blend.outputs["Color"]


def mesh(name, vertices, faces, mat, collection):
    data = bpy.data.meshes.new(name)
    data.from_pydata(vertices, [], faces)
    data.update()
    obj = bpy.data.objects.new(name, data)
    collection.objects.link(obj)
    obj.data.materials.append(mat)
    for poly in data.polygons:
        poly.use_smooth = len(poly.vertices) == 4
    return obj


def outline(width, depth, radius, count=24):
    points = []
    for cx, cy, start in ((width / 2 - radius, depth / 2 - radius, 0),
                          (-width / 2 + radius, depth / 2 - radius, 90),
                          (-width / 2 + radius, -depth / 2 + radius, 180),
                          (width / 2 - radius, -depth / 2 + radius, 270)):
        for i in range(count):
            angle = math.radians(start + i * 90 / (count - 1))
            points.append((cx + radius * math.cos(angle), cy + radius * math.sin(angle)))
    return points


def profiles(name, rings, mat, collection, caps=True):
    # Rings run around a cross-section counter-clockwise, then along its skin.
    count = len(rings[0])
    vertices = [p for ring in rings for p in ring]
    faces = []
    for j in range(len(rings) - 1):
        for i in range(count):
            a, b = j * count + i, j * count + (i + 1) % count
            faces.append((a, b, b + count, a + count))
    if caps:
        faces.extend((tuple(reversed(range(count))), tuple(range(len(vertices) - count, len(vertices)))))
    return mesh(name, vertices, faces, mat, collection)


def rounded(name, sections, mat, collection):
    return profiles(name, [[(x, y, z) for x, y in outline(w, d, r)] for w, d, r, z in sections], mat, collection)


def close_profile_ends(obj, count):
    """Close a hollow skin with an annulus, leaving its inner surface open."""
    data = obj.data
    vertices = [tuple(v.co) for v in data.vertices]
    faces = [tuple(p.vertices) for p in data.polygons]
    end = len(vertices) - count
    faces.extend((end + i, end + (i + 1) % count, (i + 1) % count, i) for i in range(count))
    data.clear_geometry()
    data.from_pydata(vertices, [], faces)
    for polygon in data.polygons:
        polygon.use_smooth = True
    data.update()


def box(name, size, position, radius, mat, collection):
    w, d, h = size
    obj = rounded(name, [(w, d, radius, -h / 2), (w, d, radius, h / 2)], mat, collection)
    obj.location = position
    bevel = obj.modifiers.new("Soft molded edges", "BEVEL")
    bevel.width = min(0.0007, h / 4)
    bevel.segments = 3
    obj.modifiers.new("Corner normals", "WEIGHTED_NORMAL")
    return obj


def curve(name, points, radius, mat, collection, cyclic=False):
    data = bpy.data.curves.new(name, "CURVE")
    data.dimensions = "3D"
    data.resolution_u = 12
    data.bevel_depth = radius
    data.bevel_resolution = 3
    spline = data.splines.new("POLY")
    spline.points.add(len(points) - 1)
    for p, co in zip(spline.points, points):
        p.co = (*co, 1)
    spline.use_cyclic_u = cyclic
    obj = bpy.data.objects.new(name, data)
    collection.objects.link(obj)
    data.materials.append(mat)
    return obj


def build():
    # This is a standalone builder, never an operation on the owner's open file.
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.name = "Dockstream 2 - Water"
    scene.unit_settings.system = "METRIC"
    scene.unit_settings.length_unit = "MILLIMETERS"
    scene["wellfed_product"] = "dockstream2"
    scene["reference_envelope_mm"] = "246 x 200 x 198; PETLIBRO published specifications"
    scene["evidence"] = "Modeled exterior from owner photographs; details estimated, not a manufacturing model."
    scene["front"] = "-Y; Z up; meters"
    collections = {}
    for name in ("Dock", "Tank", "Tray", "Fountain", "Indicators", "Water"):
        col = bpy.data.collections.new(name)
        scene.collection.children.link(col)
        collections[name] = col

    charcoal, shader = material("DS_CharcoalSatin", (0.014, 0.014, 0.014), 0.40)
    shader.inputs["Specular IOR Level"].default_value = 0.40
    # The photographs show a neutral graphite with a sandblasted, speckled
    # texture of roughly 0.3 mm: each fleck catches light as a tiny facet.
    grain = metric_noise(charcoal, "Sandblasted 0.3 mm grain", (3300, 3300, 3300))
    flecks = metric_noise(charcoal, "Fine facet flecks", (9000, 9000, 9000), 1)
    texture = blend_fields(charcoal, grain, flecks, 0.45)
    tone = charcoal.node_tree.nodes.new("ShaderNodeValToRGB")
    tone.color_ramp.elements[0].position = 0.30
    tone.color_ramp.elements[0].color = (0.011, 0.011, 0.011, 1)
    tone.color_ramp.elements[1].position = 0.75
    tone.color_ramp.elements[1].color = (0.017, 0.017, 0.017, 1)
    charcoal.node_tree.links.new(texture, tone.inputs["Fac"])
    charcoal.node_tree.links.new(tone.outputs["Color"], shader.inputs["Base Color"])
    mapped_field(charcoal, texture, 0.35, 0.45, shader.inputs["Roughness"])
    surface_bump(charcoal, texture, shader, 0.000018, 0.35)
    black, bs = material("DS_BaseBlack", (0.0065, 0.0065, 0.0065), 0.28)
    bs.inputs["Specular IOR Level"].default_value = 0.35
    base_grain = metric_noise(black, "Fine molded base", (7000, 7000, 7000))
    mapped_field(black, base_grain, 0.26, 0.36, bs.inputs["Roughness"])
    surface_bump(black, base_grain, bs, 0.000010, 0.16)
    # Fine molded plastic underneath the separate wet film. Broad satin
    # highlights preserve the charcoal color in the dry close-up.
    gloss, gs = material("DS_FountainSatin", (0.0035, 0.0035, 0.0035), 0.27)
    gs.inputs["Specular IOR Level"].default_value = 0.35
    gloss_grain = metric_noise(gloss, "Fine molded fountain grain", (4200, 4200, 4200))
    mapped_field(gloss, gloss_grain, 0.24, 0.30, gs.inputs["Roughness"])
    surface_bump(gloss, gloss_grain, gs, 0.000006, 0.18)
    steel, ss = material("DS_StainlessSteel", (0.62, 0.62, 0.62), 0.22, 1)
    ss.inputs["Anisotropic"].default_value = 0.45
    polish = metric_noise(steel, "Fine longitudinal polishing", (60, 18000, 400), 1)
    steel_cloud = metric_noise(steel, "Subtle steel finish variation", (170, 170, 170))
    steel_finish = blend_fields(steel, polish, steel_cloud, 0.30)
    mapped_field(steel, steel_finish, 0.19, 0.26, ss.inputs["Roughness"])
    surface_bump(steel, polish, ss, 0.0000004, 0.04)
    ink, _ = material("DS_IndicatorPrint", (0.43, 0.47, 0.48), 0.5)
    rubber, _ = material("DS_Rubber", (0.004, 0.005, 0.006), 0.65)
    water, ws = material("DS_ClearWater", (1.0, 1.0, 1.0), 0.025)
    ws.inputs["Transmission Weight"].default_value = 1
    ws.inputs["IOR"].default_value = 1.333
    wood, wood_shader = material("DS_WalnutVeneer", (0.10, 0.075, 0.062), 0.50)
    wood_shader.inputs["Specular IOR Level"].default_value = 0.22
    nodes, links = wood.node_tree.nodes, wood.node_tree.links
    # The dock has a printed walnut finish, not deeply carved wood. All faces
    # share a metric projection, so the thin chamfer cannot stretch the grain.
    fibers = metric_noise(wood, "Walnut longitudinal fibers", (4200, 65, 500), 3, warp=True)
    pores = metric_noise(wood, "Fine elongated pores", (16000, 320, 1600), 2, warp=True)
    density = metric_noise(wood, "Patchy pore density", (110, 180, 100), 2)
    pore_mask = nodes.new("ShaderNodeMath")
    pore_mask.operation = "MULTIPLY"
    links.new(pores, pore_mask.inputs[0])
    links.new(density, pore_mask.inputs[1])
    slow = metric_noise(wood, "Slow walnut tone", (130, 16, 40), 2)
    grain = blend_fields(wood, fibers, pore_mask.outputs[0], 0.32)
    grain = blend_fields(wood, grain, slow, 0.28)
    ramp = nodes.new("ShaderNodeValToRGB")
    # A cool, gray-brown printed walnut: dark umber streaks over a pale
    # taupe, much less saturated than natural oiled walnut.
    ramp.color_ramp.elements[0].position = 0.30
    ramp.color_ramp.elements[0].color = (0.020, 0.013, 0.010, 1)
    ramp.color_ramp.elements[1].position = 0.72
    ramp.color_ramp.elements[1].color = (0.043, 0.029, 0.022, 1)
    links.new(grain, ramp.inputs["Fac"])
    links.new(ramp.outputs["Color"], wood_shader.inputs["Base Color"])
    mapped_field(wood, grain, 0.43, 0.51, wood_shader.inputs["Roughness"])
    surface_bump(wood, pores, wood_shader, 0.000003, 0.08)

    rounded("Dock_LowerShell", [(0.240, 0.194, 0.053, 0.001), (0.245, 0.199, 0.055, 0.004),
                               (0.246, 0.200, 0.056, 0.013), (0.245, 0.199, 0.055, 0.015)], black, collections["Dock"])
    rounded("Dock_WalnutTop", [(0.243, 0.197, 0.054, 0.015), (0.245, 0.199, 0.055, 0.0158),
                              (0.245, 0.199, 0.055, 0.0192), (0.2438, 0.1978, 0.0544, 0.0205)], wood, collections["Dock"])
    rounded("Dock_TankSeat", [(0.225, 0.174, 0.047, 0.020), (0.225, 0.174, 0.047, 0.022)], rubber, collections["Dock"])
    # The open reservoir has a real inner wall and bottom; no unseen electronics.
    tank = rounded("Tank_Reservoir", [(0.226, 0.178, 0.047, 0.022), (0.230, 0.182, 0.048, 0.025),
                               (0.234, 0.186, 0.050, 0.155), (0.234, 0.186, 0.050, 0.159),
                               (0.230, 0.182, 0.048, 0.159), (0.229, 0.181, 0.048, 0.154),
                               (0.224, 0.176, 0.046, 0.028)], charcoal, collections["Tank"])
    normals = tank.modifiers.new("Stable normals across the molded lower edge", "WEIGHTED_NORMAL")
    normals.keep_sharp = True
    normals.weight = 100
    # Both side photographs show a fine vertical mold split beneath the clip.
    for side in (-1, 1):
        curve("Tank_MoldSplit_Left" if side < 0 else "Tank_MoldSplit_Right",
              [(side * (0.115 + 0.002 * t), 0, 0.027 + 0.127 * t)
               for t in [i / 32 for i in range(33)]], 0.000055, black, collections["Tank"])
    # The steel pressing is one closed skin, including its shallow well and rolled rim.
    rounded("Tray_StainlessPressing", [(0.226, 0.178, 0.047, 0.150), (0.231, 0.183, 0.049, 0.157),
                                     (0.234, 0.186, 0.050, 0.1592), (0.234, 0.186, 0.050, 0.160),
                                     (0.232, 0.184, 0.049, 0.1605), (0.229, 0.181, 0.048, 0.159),
                                     (0.223, 0.175, 0.046, 0.152), (0.216, 0.168, 0.044, TRAY_FLOOR_Z)], steel, collections["Tray"])
    for side in (-1, 1):
        obj = box("Clip_Left" if side == -1 else "Clip_Right", (0.0038, 0.017, 0.017),
                  (side * 0.118, 0, 0.1545), 0.0015, charcoal, collections["Tray"])
        # The side photo shows an actual rectangular lower pocket, bounded by
        # two rails and a bridge, with the tank wall recessed behind it.
        for edge in (-1, 1):
            box(obj.name + f"_Rail_{edge}", (0.0038, 0.0026, 0.0085),
                (side * 0.118, edge * 0.0072, 0.14175), 0.0007, charcoal, collections["Tray"])
        box(obj.name + "_LowerBridge", (0.0042, 0.017, 0.002),
            (side * 0.1182, 0, 0.1385), 0.0007, charcoal, collections["Tray"])
        box(obj.name + "_TopHook", (0.008, 0.017, 0.0032), (side * 0.115, 0, 0.162), 0.0015, black, collections["Tray"])

    # The skirt seats level in the tray. Its upper deck is a rounded ramp,
    # rear-high/front-low, while the stem itself stays upright.
    cover_rings = [[(x, y, z) for x, y in outline(w, d, r)]
                   for w, d, r, z in [(0.102, 0.069, 0.019, TRAY_FLOOR_Z),
                                     (0.104, 0.071, 0.020, TRAY_FLOOR_Z + 0.0012),
                                     (0.104, 0.071, 0.020, 0.157),
                                     (0.104, 0.071, 0.020, 0.160),
                                     (0.099, 0.066, 0.020, 0.164)]]
    deck_edge = outline(0.088, 0.056, 0.018)
    for j in range(24, 0, -1):
        fraction = j / 24
        z = 0.1645 + 0.0015 * (1 - fraction**2)
        cover_rings.append([(x * fraction, y * fraction, z) for x, y in deck_edge])
    slope = math.tan(math.radians(8))
    cover_rings = [[(x, y, z + y * slope * max(0, min(1, (z - 0.157) / 0.0075)))
                    for x, y, z in ring] for ring in cover_rings]
    profiles("Fountain_FilterCover", cover_rings, gloss, collections["Fountain"])
    # A rounded triangular mouth flares from an oval stem. Its front lip dips
    # into the waterfall; the inner profiles leave an actual throat opening.
    rings = []
    count = 128
    sections = [(0.013, 0.011, 0.160, 0), (0.012, 0.010, 0.166, 0),
                (0.011, 0.009, 0.181, 0.01), (0.014, 0.012, 0.190, 0.06),
                (0.022, 0.018, 0.1975, 0.12), (0.0225, 0.0185, 0.198, 0.12),
                (0.0205, 0.0165, 0.197, 0.12), (0.014, 0.011, 0.189, 0.05),
                (0.007, 0.006, 0.184, 0), (0.005, 0.004, 0.182, 0),
                (0.005, 0.004, 0.160, 0)]
    for rx, ry, z, triangle in sections:
        flare = max(0, min(1, (z - 0.181) / 0.016))
        rx *= 1 + 0.15 * flare
        ry *= 1 + 0.10 * flare
        ring = []
        for i in range(count):
            a = 2 * math.pi * i / count
            factor = 1 + triangle * math.cos(3 * (a + math.pi / 2))
            y = ry * math.sin(a) * factor
            # The whole mouth faces forward: a pitched rim, not just a notch.
            # Blend into the upright neck and keep the rear rim at its height.
            dip = flare * (0.018 - y) * math.tan(math.radians(12))
            dip += max(0, -math.sin(a)) ** 8 * flare * 0.0008
            # Photo proportions leave about 38 mm of
            # exposed spout, within the same published 198 mm total height.
            height = 0.164 + (z - dip - 0.160) * 34 / 38
            ring.append((rx * math.cos(a) * factor, y - 0.004 * flare, height))
        rings.append(ring)
    spout = profiles("Fountain_FlaredSpout", rings, gloss, collections["Fountain"], False)
    rings_for_mouth = (rings[6], rings[7])
    # Join the bottom annulus so the hollow molded piece is a closed manifold.
    close_profile_ends(spout, len(rings[0]))
    for p in spout.data.polygons:
        p.use_smooth = True
    subdiv = spout.modifiers.new("Smooth molded flare", "SUBSURF")
    subdiv.levels = 2
    subdiv.render_levels = 2

    # Small screen-printed glyphs sit on the front wall. LEDs stay unlit.
    col = collections["Indicators"]
    def icon(name, coords, x):
        return curve(name, [(x + u, -0.09115, 0.030 + v) for u, v in coords], 0.00013, ink, col)
    capsule = outline(0.004, 0.0026, 0.0012, count=12)
    icon("Icon_Filter", capsule + capsule[:1], -0.0225)
    icon("Icon_FilterEye", [(0.00045 * math.cos(i * math.tau / 40),
                             0.0003 + 0.00045 * math.sin(i * math.tau / 40)) for i in range(41)], -0.0225)
    icon("Icon_FilterStem", [(0, -0.0014), (0, 0.0004)], -0.0225)
    icon("Icon_Water", [(0.0017 * math.sin(t) * math.sin(t / 2), 0.002 * math.cos(t))
                        for t in [i * math.tau / 80 for i in range(81)]], -0.0075)
    battery = [(x, y - 0.00015) for x, y in outline(0.0022, 0.0033, 0.00025, count=8)]
    icon("Icon_Battery", battery + battery[:1], 0.0075)
    icon("Icon_BatteryCap", [(-0.0005, 0.002), (0.0005, 0.002)], 0.0075)
    icon("Icon_BatteryBolt", [(0.0003, 0.001), (-0.0004, 0), (0.0003, 0), (-0.0003, -0.001)], 0.0075)
    for i, r in enumerate((0.001, 0.002, 0.003)):
        icon(f"Icon_WiFiArc{i}", [(r * math.sin(a), r * math.cos(a) - 0.0015)
                                 for a in [-0.8 + j * 1.6 / 24 for j in range(25)]], 0.0225)
    for i, x in enumerate((-0.0225, -0.0075, 0.0075, 0.0225)):
        box(f"LED_Window_{i}", (0.0008, 0.0032, 0.00015), (x, -0.094, 0.02055), 0.0003, black, col)

    # Water is a removable illustration layer, not a fluid simulation.
    boundary = outline(0.219, 0.171, 0.045)
    pool_rings = [[(x, y, 0.152) for x, y in boundary]]
    for j in range(80, 0, -1):
        fraction = j / 80
        ring = []
        for x, y in boundary:
            x, y = x * fraction, y * fraction
            distance = math.hypot(x, y + 0.039)
            ripple = 0.00032 * math.sin(distance * 1150) * math.exp(-distance * 19)
            ripple += 0.00012 * math.sin(x * 310 + y * 190)
            ring.append((x, y, 0.154 + ripple))
        pool_rings.append(ring)
    pool = profiles("Water_TraySurface", pool_rings, water, collections["Water"])
    pool["evidence"] = "Modeled still water; decorative shader ripples, not flow simulation"
    pool_water = water.copy()
    pool_water.name = "DS_RippledTrayWater"
    pool.data.materials[0] = pool_water
    pool_shader = next(n for n in pool_water.node_tree.nodes if n.type == "BSDF_PRINCIPLED")
    nodes, links = pool_water.node_tree.nodes, pool_water.node_tree.links
    noise = nodes.new("ShaderNodeTexNoise")
    noise.inputs["Scale"].default_value = 110
    noise.inputs["Detail"].default_value = 2
    bump = nodes.new("ShaderNodeBump")
    bump.inputs["Strength"].default_value = 0.16
    bump.inputs["Distance"].default_value = 0.00015
    links.new(noise.outputs["Fac"], bump.inputs["Height"])
    links.new(bump.outputs["Normal"], pool_shader.inputs["Normal"])
    # The wet bowl contains a level water surface, rather than an empty throat.
    # Intersect each inner funnel profile with the spill level at the front lip.
    spill = min(rings_for_mouth[0], key=lambda p: p[1])
    start = spill[2] + 0.00025
    mouth_edge = []
    for lip, lower in zip(rings_for_mouth[0], rings_for_mouth[1]):
        fraction = max(0, min(1, (start - lower[2]) / (lip[2] - lower[2])))
        mouth_edge.append((lower[0] + (lip[0] - lower[0]) * fraction,
                           lower[1] + (lip[1] - lower[1]) * fraction, start))
    throat = [(0.004 * math.cos(i * math.tau / 128), 0.003 * math.sin(i * math.tau / 128), 0.1845)
              for i in range(128)]
    profiles("Water_MouthSurface", [throat, mouth_edge], water, collections["Water"])
    rings = []
    for j in range(49):
        t = j / 48
        flow = max(0, (t - 0.12) / 0.88)
        z = start + (0.1605 - start) * flow
        y = (spill[1] + 0.002 - 0.0028 * t / 0.12 if t < 0.12
             else spill[1] - 0.0008 - 0.005 * flow + 0.001 * math.sin(flow * math.pi))
        width = 0.0035 - 0.001 * math.sin(flow * math.pi) + 0.001 * flow**6
        width += 0.00015 * math.sin(t * 18)
        rings.append([(width * math.cos(a), y + 0.0003 * math.sin(a), z + 0.0001 * math.sin(a * 3 + t * 14))
                      for a in [i * math.tau / 32 for i in range(32)]])
    profiles("Water_FallingSheet", rings, water, collections["Water"])
    # A mildly disturbed film spreads highlights across the sloping cover.
    film_water = water.copy()
    film_water.name = "DS_CoverWaterFilm"
    film_shader = next(n for n in film_water.node_tree.nodes if n.type == "BSDF_PRINCIPLED")
    film_shader.inputs["Roughness"].default_value = 0.12
    film_noise = metric_noise(film_water, "Shallow flowing film", (220, 700, 220))
    surface_bump(film_water, film_noise, film_shader, 0.00005, 0.12)
    upper_skin = cover_rings[4:]
    film_rings = [[(x * 1.001, y * 1.001, z + 0.00020) for x, y, z in ring] for ring in upper_skin]
    film_rings += [[(x * 1.001, y * 1.001, z + 0.00005) for x, y, z in ring]
                   for ring in reversed(upper_skin)]
    film = profiles("Water_CoverFilm", film_rings, film_water, collections["Water"], False)
    close_profile_ends(film, len(film_rings[0]))
    film["evidence"] = "Modeled wet film, not a flow prediction"

    for x in (-0.084, 0.084):
        for y in (-0.062, 0.062):
            box(f"Dock_Foot_{x}_{y}", (0.018, 0.014, 0.002), (x, y, 0.001), 0.005, rubber, collections["Dock"])
    # Cable is separate, omitted from clean product cards by default.
    cable = curve("PowerCable_Approximate", [(0, 0.097, 0.009), (0, 0.112, 0.004),
                                           (0.015, 0.127, 0.002), (0.038, 0.136, 0.002)], 0.0015, black, collections["Dock"])
    cable.hide_render = True
    cable.hide_set(True)
    cable["evidence"] = "Approximate cable path; connector and underside unobserved"
    scene["unmodeled"] = "Electronics, pump, filter cartridge, exact power connector, underside fasteners and markings"
    # Closed skins traverse both ascending and descending profiles. Normalize
    # outward winding so water refraction sees air outside and water inside.
    for obj in scene.objects:
        if obj.type == "MESH":
            bm = bmesh.new()
            bm.from_mesh(obj.data)
            bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
            bm.to_mesh(obj.data)
            bm.free()
    bpy.ops.object.select_all(action="DESELECT")
    for area in bpy.context.screen.areas:
        if area.type == "VIEW_3D":
            area.spaces.active.region_3d.view_distance = 0.43
            area.spaces.active.region_3d.view_location = Vector((0, 0, 0.095))
            area.spaces.active.region_3d.view_rotation = Vector((-0.5, -0.8, 0.45)).to_track_quat("Z", "Y")
            area.spaces.active.shading.color_type = "MATERIAL"
    bpy.context.preferences.filepaths.save_version = 0
    # Share the product collections, so an edit to the tank or spout remains
    # identical in both scenes. The dry scene never links water geometry.
    dry_scene = bpy.data.scenes.new("Dockstream 2 - Dry")
    dry_scene.unit_settings.system = "METRIC"
    dry_scene.unit_settings.length_unit = "MILLIMETERS"
    for key in scene.keys():
        dry_scene[key] = scene[key]
    dry_scene["water_state"] = "dry"
    for name, collection in collections.items():
        if name != "Water":
            dry_scene.collection.children.link(collection)
    bpy.ops.wm.save_as_mainfile(filepath=str(HERE / "dockstream2.blend"))
    print(f"Built Dockstream 2: {len(scene.objects)} editable objects, water and dry scenes")


if __name__ == "__main__":
    build()
