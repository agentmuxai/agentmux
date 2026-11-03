# The AgentMux app icon

Two flat sources, each a square tile and four square blocks (a pane grid), no gradients:

| File | Use |
|---|---|
| `agentmux-dark.svg` | The default: a dark tile, blocks in one blue-violet. Wherever only one icon is possible (Linux, the portable exe, the tray) this is the one. |
| `agentmux-light.svg` | The light-theme one: a light tile with a darker edge (so it shows on a white taskbar), blocks in a deeper blue-violet. |

Both sit on a 16 px grid: on a 512 unit canvas every edge is a multiple of 32 units, so it lands on a whole pixel at 16, 32, 48 ... px. Margin 2 px, gap 2 px, bar 2 px high, blocks 5 x 8 and 5 x 3 (at 16 px). Keep it that way when you change them.

## Regenerating

```
node scripts/gen-app-icons.mjs           # write every raster
node scripts/gen-app-icons.mjs --check   # fail if a committed file isn't what the SVGs draw
```

No image tools needed: the script reads the rectangles from the SVGs and draws them itself, so it writes the same bytes on every machine. A test (`scripts/gen-app-icons.test.mjs`) fails when a committed icon is out of date.

## What it writes

- Linux: `assets/linux/icons/hicolor/<size>/apps/agentmux.png` (16 to 512), the dark icon.
- Windows: `crates/cef/resources/win/agentmux.ico` (16, 24, 32, 48, 64, 256), the dark icon, used by the exe, the launcher, srv and mcp.
- The frontend's `frontend/app/asset/agentmux-icon.png` (256), and the tray and notification PNGs `assets/favicon-{71,150,300}x*.png`.
- MSIX (`packaging/msix/assets/`): the Store and tile logos, and `Square44x44Logo.targetsize-N_altform-unplated.png` (dark tile) and `..._altform-lightunplated.png` (light tile) for the taskbar. Windows picks the light-taskbar one when the taskbar is light.
- macOS: `assets/macos/agentmux-1024.png`, the four blocks on Apple's rounded template (an 824 px rounded square with corner radius 185 on a 1024 canvas, so the Dock's spacing holds). `scripts/package-macos.sh` builds `AgentMux.icns` from it. macOS rounds the tile itself, so a square tile would sit wrong in the Dock; the blocks stay square. `crates/launcher/resources/menubar-template.png` is the blocks alone, black on transparent (macOS colours a template image from its alpha).
- The web set, `assets/icon/web/` (for the landing and docs sites, which live in other repos): `favicon.svg` (the dark icon, and the light one when the browser is in dark mode, since its tab strip is dark), `favicon-32.png` and `favicon-32-dark-mode.png`, `apple-touch-icon-180.png`, `favicon.ico`. Use them as:

```html
<link rel="icon" href="/favicon.svg" type="image/svg+xml">
<link rel="icon" href="/favicon.ico" sizes="48x48">  <!-- Safari and others without SVG favicons -->
<link rel="apple-touch-icon" href="/apple-touch-icon-180.png">
```
