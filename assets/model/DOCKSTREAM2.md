# Dockstream 2 model

This Blender asset models the visible exterior of the owner's charcoal PETLIBRO Dockstream 2 fountain with a walnut-colored dock. It is for product illustrations, not manufacturing or fit checks. Adding the artwork does not establish firmware support for the fountain.

The model uses meters, Z up, with the front toward −Y. The nominal envelope is 246 × 200 × 198 mm, from [PETLIBRO's specifications](https://petlibro.com/products/dockstream-2-smart-fountain). Individual dimensions and curves are estimated from photographs, not physical measurements.

## Parts

The file contains only the product, without cameras, lights or a stage. Collections separate the dock, reservoir, stainless tray and clips, fountain cover and hollow spout, indicators, and water. Materials are procedural and need no external textures. The owner's photographs are not embedded.

The four front glyphs follow the owner's photograph: filter, water, battery, and Wi-Fi. Indicator windows are unlit. The vendor wordmark is omitted, as in Wellfed's other rendered assets.

The file has two scenes: `Dockstream 2 - Water` and `Dockstream 2 - Dry`. They share the product collections, so edits to the body or spout appear in both. The dry scene does not link the water collection.

Water is an editable illustration layer, including a rippled tray surface, smooth wet cover, water within the upper mouth and a narrow falling sheet. The ripples and flow are modeled geometry, not a fluid simulation or evidence of flow behavior. The smooth film and stream use a separate material from the tray's rippled water.

The power cable is approximate and hidden by default. The exact connector, underside fasteners, electronics, pump and filter cartridge are not modeled. Internal wall and tray thicknesses are estimates.

The tank and tray proportions follow the front and dry reference photographs: the steel rim sits about 160.5 mm above the ground, leaving about 38 mm of exposed spout. The broad footprint curves use modeled outer radii of 50 mm for the tank/tray and 56 mm for the dock. These are visual fitting values, not recovered dimensions. The smooth flare, side mold seams, satin body texture, polished steel and muted walnut grain are modeled from the visible exterior.

The filter cover has a curved upper deck sloping down toward the front, with a level lower skirt seated directly on the steel tray floor. Their modeled contact height is shared, with no air gap. The upper deck's modeled slope is about 8°, estimated from the app image and close-up reference rather than measured. The stem remains upright and the wet film follows the sloping deck. The `spout-side` view makes this profile visible.

Surface textures use object coordinates in meters. The body is a neutral graphite with a sandblasted, speckled grain of roughly 0.3 mm, carried in its color, roughness and bump. The cover and spout use fine-grained charcoal satin plastic. A separate, slightly disturbed water film supplies the wet reflections. The printed walnut is a subdued brown with fine, low-contrast fibers running front to back across the dock. Its shallow relief represents a printed surface rather than carved wood. The steel is brushed rather than mirror-polished, with directional marks and subtle roughness variation. These are reconstructed finishes, not textures sampled from the owner's photographs.

## Rebuild and render

Run from the repository root with Blender 5.2:

```sh
blender -b --python-exit-code 1 -P assets/model/dockstream2.py
blender -b assets/model/dockstream2.blend -P assets/model/render.py -- --all
```

Rebuilding replaces this generated model, including both scenes. Keep manual edits in a separate file or transfer them into the builder before rebuilding.

The shared renderer recognizes the fountain model and writes to `assets/renders/dockstream2/`, leaving the feeder's renders separate. Views are `card`, `app`, `front`, `back`, `top`, `spout`, `spout-side`, `materials` and `clip`. The last two show the lower shell and dock grain, and the retaining clip's recessed pocket. Each has a transparent PNG with a contact shadow and a `-cutout` copy. `--all` includes matching `-dry` images. For one dry view:

```sh
blender -b assets/model/dockstream2.blend -P assets/model/render.py -- --view spout --state dry
```

For a 4096 px Cycles render with 512 samples and denoising:

```sh
blender -b assets/model/dockstream2.blend -P assets/model/render.py -- --view card --state dry --size 4096 --samples 512 --out assets/renders/dockstream2/4k
```

The `app` view faces the front-right at a shallower elevation than `card`, with a broad white studio reflection to reproduce the pale steel in the vendor app reference. Its framing and lighting are comparison choices, not recovered camera settings.

The committed render set contains 36 PNGs at 1024 × 1024: all nine views, wet and dry, with contact-shadow and cutout variants. The `4k/` folder adds six PNGs at 4096 × 4096: wet `card`, dry `card` and dry `app`, each with its cutout. All use Cycles at 256 samples with denoising and the same model materials.

## Evidence

- **Modeled, November 9, 2026.** Exterior proportions, finishes, clips, glyphs and spout were reconstructed from the owner's supplied front, side and close-up photographs. The published envelope supplies scale; no physical measurements were taken.
- **Simulation, November 9, 2026.** Cycles renders provide product and close-up views. They do not establish hardware behavior or dimensional fit.
- **Modeled and reviewed, November 9, 2026.** Three independent reviewer passes checked the finishes, then both geometry and finishes against the photographs. Corrections include the clip pockets, tank corner normals, flat indicator windows, wood grain and polishing strength. The final pass found no remaining must-fix illustration issues; it did not establish dimensional fit.
- **Static analysis, November 9, 2026.** Both scenes share the product collections; the dry scene excludes water. All 28 mesh objects have closed skins and positive signed volume, including evaluated modifiers. The nominal geometry spans 246 × 200 × 198 mm. This is a mesh check, not dimensional validation against a physical unit.
- **Modeled, November 10, 2026.** The owner's app screenshot and white-device tray close-up exposed broader footprint curves, a wider mouth flare and a sloping filter-cover deck that the earlier illustration review had not resolved. Those references informed the current geometry and the app comparison view; their small images do not establish exact dimensions or slope.
- **Static analysis, November 10, 2026.** Both scenes share the product geometry, and the dry scene excludes the four water objects. All 29 mesh objects have closed skins and positive signed volume, including evaluated modifiers. The water film is a thin closed skin following the inclined deck.
- **Modeled contact, November 10, 2026.** The cover underside and steel floor share a height, giving a 0 mm modeled gap. Its flat contact footprint spans at least 97% of the cover's maximum width and depth. This corrects the visible clearance in an earlier render; it does not establish physical fit.
- **Modeled, November 10, 2026.** The owner judged the finishes still off. Compared with the photographs, the body was too dark and smooth, the walnut too orange with its grain turned the wrong way, the cover too mirror-like and the steel too polished. All four were retuned against close crops of the photographs; colors are judged by eye under studio light, not measured.

- **Modeled and reviewed, November 10, 2026.** The upper spout now slopes toward the front across the whole mouth, with a modeled 12° pitch and 4 mm forward offset blended into an upright neck. The water level and stream origin follow the revised pouring lip. These values are visual estimates. Dry plastic grain, wood contrast and shell relief were retuned; lower studio lights reduce the large white reflection that made the wet cover appear metallic. The water remains a static illustration.

- **Modeled, November 10, 2026.** A dedicated color pass against the app and marketing references neutralized and darkened the charcoal, deepened the spout black, reduced walnut grain contrast and relief, and softened steel microtexture. Before/after renders use identical lighting; this is visual color fitting, not calibrated color measurement.
