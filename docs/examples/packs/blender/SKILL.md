---
name: blender
description: Use when you need to make, change or render 3D scenes in Blender (blender.exe on Windows, Blender on macOS and Linux): objects, materials, cameras, renders to image or video files. Covers headless Python (bpy) first, then a pinned BlenderMCP only if the person approved it, then shortcuts, then Desktop clicks, with a check after each step.
---

# Blender

**Status:** active — verified on Windows with Blender 5.2.3 LTS (see Proof); the shortcut and Desktop routes are not verified.

Pack: `blender` v0.1.0. Executables: `blender.exe` (Windows), `blender` (macOS, Linux). The Python route was run on Windows with Blender 5.2.3 LTS on 2026-11-05 and passed (see Proof); the shortcut and Desktop routes are not verified.

## The ladder: use the first route that works

1. **Headless Python (`bpy`).** No window, no grant, same on every platform. Run a script with Blender in the background:
   `blender --background --factory-startup -noaudio --python-exit-code 1 --python script.py -- <your args>`
   - `--background` (`-b`) runs without a window. `--factory-startup` ignores the user's own startup file, so the run is reproducible. `--python-exit-code 1` makes an exception in your script a non-zero exit code: without it, a failed script still exits 0.
   - Arguments after `--` are yours: read them from `sys.argv[sys.argv.index("--") + 1:]`.
   - **Order matters: flags run in the order given.** Blender's own help: `--background test.blend --render-output /tmp --render-frame 1` works; putting `--render-frame 1` before the output path renders before the path is set, and a blend file loaded after `--render-output` overwrites it. Put the `.blend` first, then `-o`, `-F`, `-E`, then `-f` (one frame) or `-a` (animation).
   - To render an existing file: `blender -b scene.blend -o //out_#### -F PNG -f 1`. `//` means next to the blend file; `####` becomes the frame number.
   - Start a new scene from nothing with `bpy.ops.wm.read_factory_settings(use_empty=True)`.
   - Install: the official portable zip from `download.blender.org/release/Blender<x.y>/`, with its `.sha256` file checked, or the platform package manager (`BlenderFoundation.Blender` in winget). Do not take it from a mirror.
2. **A BlenderMCP add-on**, only if the person approved that exact version. It runs Python you send it inside Blender over a local socket. Its `execute_blender_code` tool has a published code-injection flaw (CVE-2026-10688): anything that can reach the socket runs code as you. Prefer route 1; if you must, pin the version, run it where nothing else can connect, and never expose it beyond the machine. A pack never installs it.
3. **Shortcuts** in the Blender window (Blender's default keymap, not verified here): add `Shift+A`, grab, rotate, scale `G` `R` `S`, edit mode `Tab`, delete `X`, render image `F12`, render animation `Ctrl+F12`, play `Space`, save `Ctrl+S`, side panel `N`. The keymap can be remapped by the person.
4. **`Desktop` clicks**, last. Blender draws its own widgets, so a screenshot is all `Desktop` sees; use zoom on small fields.

## Pitfalls

- **Ordering of flags** (above): the commonest cause of a render in the wrong place or of nothing rendered.
- **Version drift in `bpy`.** Names and properties move between releases (Blender 5.2 warns that `Material.use_nodes` is expected to be removed in 6.0). Do not set a property just because an old tutorial does: check it exists (`hasattr`, or read it first) and read warnings in the output.
- **Render engine ids change.** Use `-E help` for the list on this install; the test uses Cycles on the CPU with 4 samples to stay quick and to need no GPU.
- **Rendering takes time and CPU.** Keep test renders tiny (160 by 120 pixels); ask before a long render.
- **Never overwrite the person's `.blend`.** Save a copy first (`bpy.ops.wm.save_as_mainfile(copy=True, filepath=...)`).
- **Scripts run with your rights.** Do not run a script you did not write or read.
- No licence codes or passwords are ever needed; if a dialog asks, ask the person.

## Verify after each step

| Step | Cue that it worked |
|---|---|
| Blender runs | `blender --version` prints the version; the script's own print line appears |
| Script ran | exit code 0 with `--python-exit-code 1`, and no `Traceback` in the output |
| Scene built | read it back in the same script: object count, material name, camera set |
| Render | a PNG exists at the path, with the expected size (`check_png.py`) and not blank |
| Source untouched | the original `.blend` file's hash is unchanged |

## Proof

`test.json` is the pack's test; its script is `pack_test.py` and its check is `check_png.py`. Verified on: Windows, Blender 5.2.3 LTS, 2026-11-05, pass (pack v0.1.0).
