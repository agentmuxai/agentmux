# Files: the system's own file icons, and real thumbnails for images and video

**Status:** active. Approved by the operator on 2026-10-27 ("files spec approved with best recommendations"), so every §13 decision goes as recommended. Phase 1 (Windows type and folder icons) is built in the PR that commits this file (#5827); P2 to P5 are not built.
**Date:** 2026-10-27.
**Requested by:** the operator, through AgentY. Their words: "can we get a spec on supporting system icons in the files (so we see the same icons for the files as in windows explorer."; and "can we also get image and video previews as the actual file thumbnail for media file in the files? simlar to modern windows".
**Author:** Agent1@narko.
**Related:** `SPEC_FILE_BROWSER_PANE_2026_10_01.md` (the Files pane; §5.2 views, §12.5 grid), `SPEC_FILES_ZOOM_2026_10_08.md`.
**Checked against:** `origin/main` at 685c739a4 (v0.60.x).

---

## 1. Summary

Today every Files row is a Font Awesome glyph picked by a regex on the file name (`iconOf`, `files-view.tsx`), and only the grid view shows a thumbnail, and only for images: the page downloads each image (up to 25 MB), decodes it in JavaScript and scales it to 192 px (`files-thumbs.ts`). Videos, PDFs and everything else get a glyph.

The proposal, in one paragraph: **srv serves small PNGs of the icons and thumbnails the operating system itself would show, through two new routes. The Windows shell supplies them (the same calls Explorer uses, so the same icons, the same associations and Explorer's own thumbnail cache), run in a killable helper process so a third-party shell extension can't take srv down. Only pixels cross to the page. Our own decoder (the `image` crate srv already has) is the fallback and the cross-platform path.** Font Awesome icons stay as the placeholder while an icon loads, as the fallback when none can be made, and as a setting (`files:icons = system | app`). Windows ships first; macOS and Linux follow with different sources behind the same routes.

## 2. What was measured (Windows 11, this machine, throwaway script, not in the repo)

A PowerShell/C# script (`icon-proto/proto.ps1` in my workspace; contact sheet `icon-proto/sheet.png`) called the shell the way srv would: `SHGetFileInfo` with `SHGFI_USEFILEATTRIBUTES` plus `SHGetImageList` for by-name icons, and `IShellItemImageFactory::GetImage` for per-file icons and thumbnails. These facts decide the design.

| Question | Result |
|---|---|
| Can an icon be had **for a name that doesn't exist** (a file on an SSH host)? | Yes. `SHGFI_USEFILEATTRIBUTES` resolves by extension from the registry without touching a disk. Unknown extension (`.nope_zz`) gives the generic page icon in 1.3 ms. |
| Cost per distinct extension | 1.6 to 25 ms warm. The very first call in a process took 196 ms (shell and association load), so the first row of the first folder pays once. |
| Folder icon at 16 / 48 / 256 px | 2 to 15 ms each, same call path. |
| Icons follow **this machine's associations** | `.ts`, `.mp4`, `.mkv` showed the VLC cone, `.pdf` the Acrobat icon: exactly what Explorer shows here. A stock glyph set can't do that; this is the point of the ask. |
| Per-file icon (`.exe` installer) via `GetImage` + `SIIGBF_ICONONLY` | Works; 49 ms the first time for one file (it reads the file's resources), 1.6 to 4 ms for per-extension types. Per-file icons are only needed for the few "own icon" types (`.exe`, `.ico`, `.lnk`, `.url`, `.scr`, `.cur`, files whose ProgID `DefaultIcon` is `%1`, and folders customised by `desktop.ini`). |
| Thumbnail via `SIIGBF_THUMBNAILONLY` | `.jpg` 17.6 ms, `.png` 3.7 ms, `.mp4` 15.1 ms, `.mkv` **240.8 ms** cold (a decode); all about 2 to 9 ms the second time. All four were already `INCACHEONLY` hits: the shell had them in Explorer's thumbnail cache, so we reuse work Explorer did. |
| Thumbnail for `.pdf`, `.docx`, `.xlsx`, `.exe`, `.zip`, `.txt`, `.lnk` | **Fails with `0x8004B200` (no thumbnail handler)**, in 1 to 13 ms. On this machine there is no PDF or Office preview handler; on a machine with Office or a PDF reader installed the same call succeeds. We fall back to the icon and never need to know which. |
| Cache-only probe | `INCACHEONLY` answers without decoding (`0x80030002` when absent), which is the cheap "do we already have it" test. |

Not verified: the shortcut arrow on `.lnk` (Explorer draws it as an overlay; the prototype's `.lnk` output did not show it clearly), OneDrive status overlays, High-DPI scaling above 100 %, any macOS or Linux behaviour (§8, §9 say what a Mac or Linux seat must check).

## 3. What the Files pane looks like today (so the change fits it)

- Details rows are 24 px high (`ROW_HEIGHT`) with a 14 px FA glyph. Grid tiles are 112×132 with a 96×80 art box (`TILE_W`, `TILE_H`, `.files-tile-art`). Both are windowed: only visible rows exist in the DOM (`visibleNames`).
- Grid thumbnails: images only, `THUMB_PX` 192, 300 kept (`MAX_CACHED`), 4 decoded at once (`MAX_ACTIVE`), the file fetched whole through `/agentmux/stream-local-file` (auth header, so an `<img src>` can't be used directly; hence blobs and object URLs). Videos and larger images get a glyph.
- A Files can browse an SSH host (`connection` in block meta); those entries come from the remote helper and their files are not on this machine. WSL appears as `\\wsl$` paths on Windows. There is no archive browsing.
- `FsEntry` carries name, `is_dir`, `is_symlink`, size, mtime, hidden, readonly, link target, an error; no icon fields.

## 4. Design

### 4.1 Two routes, one cache

Both served by srv, same instance-key auth as `/agentmux/stream-local-file`, `GET`, response `image/png` (or `image/svg+xml` on Linux, §9), `Cache-Control: private, max-age=86400`, an `ETag` from the cache key.

- **`/agentmux/file-icon?kind=<file|dir|drive>&ext=<ext>&px=<n>`**: the icon for a type. No path. `ext` is validated `^[A-Za-z0-9_+-]{0,24}$` (no dot, may be empty); `px` is one of 16, 20, 24, 32, 40, 48, 64, 96, 128, 256, else 400. This is the common case: a folder of 5,000 `.ts` files is **one** request.
- **`/agentmux/file-thumb?path=<abs>&px=<n>&m=<mtime>&len=<size>`**: the picture for one file: its own icon when the type has one, else its thumbnail, else the type icon. `m` and `len` come from the listing and only make the URL change when the file does (the cache key). Needs a real, local, readable file.

The page never composes a path out of anything but a listing entry it already has, and never receives anything but pixels.

### 4.2 Who makes the pixels (Windows)

**A helper process, not srv and not the CEF host.** The calls load third-party code: icon handlers (`IExtractIcon`), thumbnail providers, codecs, cloud-provider shell extensions. Thumbnail providers usually run in the shell's own surrogate, but icon handlers load in the caller. A hung or crashing handler in srv would stall or kill the process that holds every agent's terminal; in the CEF host it would take every window with it. So:

- srv starts a small pool (2 workers) of the helper on first use and speaks a length-prefixed request/response protocol over its pipes. Request: `{kind, ext|path, px, flags}`; response: PNG bytes or an error code (`NoHandler`, `NotFound`, `Denied`, `Timeout`).
- Each request has a **3 s deadline**. On a timeout srv kills that worker by its PID, starts another, and answers `504`; the page shows the type icon and does not retry that file this session.
- The helper is `agentmux-srv` itself re-executed with a subcommand (no new binary to sign, package or update). The build must confirm `main` can dispatch a subcommand before any bootstrap; if it can't cheaply, a tiny second binary is the fallback (decision D2).
- Inside the helper: one dedicated **STA** thread with COM initialised (the shell expects it), `SHGetFileInfo`/`SHGetImageList` by name, `SHCreateItemFromParsingName` + `IShellItemImageFactory` by path, bitmap read from the DIB section as premultiplied BGRA (a plain `Image.FromHbitmap` loses alpha, found in the prototype), converted and PNG-encoded with the `image` crate. Crate surface: the `windows` crate already in `crates/srv/Cargo.toml` gains `Win32_UI_Shell`, `Win32_UI_Shell_Common`, `Win32_Graphics_Gdi` and `Win32_UI_Controls`; no new dependency.

### 4.3 Which request becomes which call

| Entry | Local fixed drive | Network drive, UNC, `\\wsl$`, removable | SSH host |
|---|---|---|---|
| File of a normal type (`.rs`, `.md`, `.pdf`) | by-extension icon (`file-icon`) | by-extension icon | by-extension icon |
| File with its own icon (`.exe`, `.ico`, `.lnk`, `.url`, `.scr`, `%1`-icon types) | `file-thumb`, ICONONLY | by-extension icon (a per-file read can block for seconds on a network share) | by-extension icon |
| Image or video, in the grid | `file-thumb`: THUMBNAILONLY, falls back to the icon | by-extension icon | on demand only (§7) |
| Folder | generic folder icon; per-folder (`file-thumb`) only when the folder has the READONLY or SYSTEM attribute, which is Explorer's own rule for reading `desktop.ini` | generic | generic |
| Drive root | `kind=drive`, per drive | n/a | n/a |
| Cloud placeholder (OneDrive "online only": `RECALL_ON_DATA_ACCESS` / `OFFLINE`) | icon only, **never a thumbnail** (a thumbnail read downloads the file) | n/a | n/a |

Whether a type has its own icon is decided once per extension in the helper from the registry (`HKCR\.ext`, its ProgID's `DefaultIcon`, an `IconHandler` key) and remembered. The page gets the set of "own icon" extensions once per session (small list) so it picks the route without a round trip per row.

### 4.4 Cache

- **srv memory:** encoded PNGs by key, LRU, 32 MB.
- **srv disk:** `<AgentMux data dir>/cache/file-icons/v1/`. Type icons: `ext/<px>/<ext>.<assoc-hash>.png` (a few hundred small files, 20 MB ceiling). Thumbnails: `thumb/<hash(path, mtime, len, px)>.png`, **256 MB LRU, 30-day TTL**, trimmed at start and when the cap is passed. A "Clear thumbnail cache" button in Settings.
- **Invalidation of type icons:** `assoc-hash` = hash of the extension's ProgID, its `DefaultIcon` string and any `IconHandler` CLSID, read from the registry on each request (three reads, microseconds). Installing VLC or changing a default app changes the hash and the next request produces a new file; no notification channel (srv has no window) is needed. Thumbnails need no invalidation: a changed file changes `mtime` or `len`, so the key changes.
- **Page:** object URLs in an LRU like today's (300 for thumbnails; type icons are bounded by the number of distinct types, capped at 200), in-flight requests shared and aborted when the last row wanting one scrolls away, at most 6 at a time, failures remembered for the session.

### 4.5 Sizes and layout (no jumping)

The `<img>` sits in the same box the glyph has today (16×16 in a row, the 96×80 art box in the grid) with `object-fit: contain`, `decoding="async"`. The glyph shows until the picture arrives, then it is swapped in the same box, so nothing reflows. `px` requested = CSS size × `devicePixelRatio` × the pane's zoom (`SPEC_FILES_ZOOM`), snapped up to the list in §4.1, so a 125 % display or a zoomed pane asks for 20 or 24 px, not a blurry 16.

### 4.6 Bigger grid sizes, so thumbnails are worth having

At 96×80, a photo thumbnail is a postage stamp. Add **grid sizes** like Explorer's: Small 64, Medium 96 (today), Large 160, Extra large 256 (tile = art + name; `TILE_W/H` follow). Chosen in the toolbar next to the grid toggle, kept in block meta (`files:grid:size`), keyboard `Ctrl+scroll` over the grid is already the zoom, so the size is a separate control. The Details list stays icon-only at 16 px: thumbnails in a 24 px row would be unreadable, and that is also Explorer's behaviour.

### 4.7 Our own decoder: fallback and the cross-platform path

For images, srv decodes and resizes with the `image` crate it already links (png, jpeg, gif, webp, bmp, tiff), in `spawn_blocking`, with decoder limits (max dimensions 16,384 × 16,384, max allocation 256 MB, file size ≤ 100 MB, so a decompression bomb costs a refusal, not the process), and caches the result under the same `thumb/` key. It is used when the shell has no handler or fails (HEIC and AVIF without the Store extension, an odd TIFF), on macOS and Linux until their native sources land, and for remote files (§7). It also **replaces the page-side decode** (`files-thumbs.ts` stops downloading 25 MB files into JavaScript; the page fetches a 5 to 20 KB PNG instead). SVG is not decoded by us: the grid already shows it as an image through the page; it keeps that.

Video poster frames are not something srv can make: there is no video decoder in the tree. On Windows the shell supplies them. Elsewhere a video keeps its type icon until a decision on a decoder (D8). A browser-engine route (`<video>` seeked to 0.1 s and drawn to a canvas) would work for mp4/webm if the CEF build has H.264, which is unconfirmed and is a page-side decode, so it is not proposed.

## 5. Theme fit

System icons are full-colour bitmaps drawn for light and dark backgrounds alike; Windows ships its own icons in both themes and they read on the Files pane's dark surfaces (the prototype sheet was drawn on `#202024`). Two rules: (1) no tinting, no CSS filters, the picture is shown as made; (2) `files:icons = app` keeps today's FA glyphs and their theme colours for anyone who prefers a monochrome list. The thumbnail frame is a 1 px border in `--border-color` so a white page thumbnail doesn't bleed into a light theme.

## 6. Security and privacy

- **Only pixels cross.** The routes return PNG (or an SVG from the icon theme on Linux, served only as an `<img>`, which cannot run script). No file contents, no path echoed back, no error text beyond a status code. Decoding and shell calls happen in the helper or in srv, never in the page.
- **Nothing the page couldn't already do.** `stream-local-file` already serves any readable local file to a page holding the instance key; the thumbnail route reads strictly less (a few KB of derived pixels) under the same authentication. It adds no new principal and no new reach.
- **No path widening:** `file-icon` takes no path at all; `file-thumb` takes an absolute path that must exist, be a regular file or folder, be local (not UNC, not network), have no NUL, be canonicalised before use, and the `px`, `ext` and `kind` are enumerated. The helper never opens a file for writing and never follows a placeholder's data.
- **Agent-opened Filess are the same code with the same limits:** an agent can navigate and select (`OpenFiles`) but cannot ask for anything the page itself can't; the routes have no agent-callable RPC.
- **Disk cache = a copy of the user's pictures at small size.** It sits under the user's own AgentMux data directory (user-only permissions on every OS), is capped and expires, can be cleared, is not synced, not backed up by AgentMux and never contains remote files (§7). Windows' own `thumbcache` already holds the same thumbnails for Explorer, so reusing it adds nothing new. A privacy-minded user sets `files:thumbnails = off` and gets icons only.
- **Third-party code isolation** is the reason for the helper (§4.2); a handler that hangs costs one 3 s timeout and one respawn.
- **Rate and size limits:** at most 6 in flight per page, 2 helper workers, queue of 64 then `503`; response bodies capped at 256×256 RGBA.

## 7. Remote and virtual locations

- **SSH host:** the file is not here. Type icons come from the **local** system by extension (`USEFILEATTRIBUTES` works for a name that doesn't exist, §2), so a `.pdf` on a server shows the PDF icon this machine would show; folders show the generic folder. It is a faithful answer to "what is this type", not to "what does that server's desktop show".
- **Remote thumbnails** are the one thing that moves bytes: **on demand only** (a per-folder "Show thumbnails" control, off by default), images only, files ≤ 2 MB, fetched through the existing remote file read, decoded by our own decoder, held **in memory only** (never written to the disk cache), cap 64 per folder. Videos and larger files keep the type icon.
- **WSL** (`\\wsl$`), **mapped drives and UNC:** by-extension icons only (§4.3); no per-file shell calls and no thumbnails, because a stalled share would hold a worker for the whole deadline.
- **Archives and cloud placeholders:** there is no archive browsing today; if added, entries are by-name. Placeholders get icons, never thumbnails (§4.3).

## 8. Performance

- **A folder of thousands of files costs one request per distinct type**, not per file. Rows already exist only when visible; each visible row asks for its type's icon (shared, usually cached) and, for the few "own icon" files, one `file-thumb`.
- **A folder of thousands of photos:** only the visible tiles ask (plus the existing 2-row overscan); each request is a queue entry that is **aborted when the tile scrolls away** (the `Job.interest` count `files-thumbs.ts` already has); 6 in flight; srv answers from memory or disk cache in under a millisecond, and cold ones cost the 4 to 18 ms measured (a cold video 240 ms). Fast scrolling therefore costs only the tiles you stop on.
- **First open of a folder:** the placeholder glyph is already on screen, the first rows' icons arrive in tens of milliseconds, the first call in a session may add about 200 ms (§2); the Files pane's budgets in `SPEC_FILE_BROWSER_PANE` §10 (first rows < 100 ms, no frame > 16 ms) are about rows, not icons, and stay as they are because icons never block a row.
- **srv stays unblocked:** all shell work is out of process; decodes are `spawn_blocking`; the route is `async`.

## 9. Other platforms (same routes, different source)

- **macOS:** `NSWorkspace` `iconForFileType:` (or `iconForContentType:` with a `UTType`) for type icons; `iconForFile:` for per-file icons (`.app` bundles, custom icons, folders); `QLThumbnailGenerator` (QuickLookThumbnailing) for thumbnails, which also covers PDF, Office via Quick Look generators, and video. Needs `objc2-app-kit` and `objc2-quartz-core`-family crates added to the `objc2` dependencies srv already has: **new supply-chain surface, flagged for review**. Runs in the same helper (a macOS subcommand), with the same deadline. Not verifiable from Windows: Clare, Masty or Maop on starpower confirm the AppKit calls work in a process with no `NSApplication`, high-DPI sizes, and the first-call cost.
- **Linux:** type icons from the user's current freedesktop icon theme (setting read from GTK settings / `kdeglobals`, falling back to `hicolor`), resolved by MIME type from shared-mime-info; thumbnails from the freedesktop thumbnail cache (`~/.cache/thumbnails`, MD5 of the file URI, `normal` 128 and `large` 256) when present, else our decoder. Icons are often SVG: served as `image/svg+xml` to an `<img>` (no rasteriser added). No new crate is required (index.theme parsing is small); `freedesktop-icons` is optional (D10). Maricon or Parlo on charlie verify GNOME and KDE themes, a missing theme, and Wayland/X11 with no desktop session (a headless box gets the app icons).
- **Cache keys on those:** macOS keys type icons by the content type and the default app's bundle id; Linux by the theme name and its directory mtime, the Windows `assoc-hash` equivalent.

## 10. Phases, owners, sizes

| Phase | What | Where | Size | Who |
|---|---|---|---|---|
| **P1** | Windows type and folder icons: `file-icon` route, helper process + pool + deadline, memory and disk cache, `files-icons.ts`, `<img>` in rows and tiles with the glyph as placeholder, `files:icons` setting, tests | srv, helper, `frontend/app/view/files` | M | the build agent (#5827); Agent1 for the page and theme side |
| **P2** | Windows per-file icons (own-icon types, custom folders, drives), shell thumbnails in the grid, grid sizes (§4.6), placeholder safety | same | M | same |
| **P3** | Own decoder: srv-side image thumbnails replacing the page-side decode; remote on-demand thumbnails; `files:thumbnails` and the cache-clear button | srv, page | M | same |
| **P4** | macOS source | helper (macOS), Cargo | M, needs a Mac | seat for Mac, verified by starpower |
| **P5** | Linux source | helper (Linux) | S to M | verified by charlie |

P1 alone answers "the same icons as Explorer" for everyday files; P2 is what makes folders like Downloads and System32 look like Explorer and gives the thumbnails.

## 11. Tests (each written to fail without its change)

- **Pure (Rust):** extension validation and the `px` list (bad values refused); the request-to-route table in §4.3 (UNC, network, placeholder, SSH); the `assoc-hash` changes when ProgID or `DefaultIcon` changes and not otherwise; the cache key changes with `mtime`/`len`; decoder limits refuse a 40,000 × 40,000 PNG header without allocating.
- **srv route:** missing/oversized/odd parameters give 400; unauthenticated gives 401; a path that is a directory, missing, UNC or contains NUL gives 400/404 without a body that names it; a second identical request is served from cache (counted); a helper that hangs yields 504 within the deadline and the next request works on a fresh worker (fake helper in the test).
- **Helper (Windows-only, `#[cfg(windows)]`):** by-name `.txt`, `.rs`, unknown extension and folder return non-empty PNGs of the requested size with non-zero alpha; a nonexistent file name works; a placeholder-attribute file refuses THUMBNAILONLY.
- **Page (vitest):** the glyph shows until the picture loads and the box size is identical before and after (no layout shift); a tile scrolled away aborts its request; failure is remembered and not retried; `files:icons = app` renders glyphs only; the grid size setting changes tile geometry and the requested `px`; the own-icon set is fetched once.
- **Live (my own isolated `task dev`, never the installed AgentMux):** one folder opened in Explorer and in a Files pane, side by side, screenshots in the PR (System32-like: many `.exe`/`.dll`; Pictures; a 3,000-file folder); scroll a 5,000-photo folder and watch the trace for frames over 16 ms; associations change (install or remap a type) and the next request shows the new icon; an SSH folder shows local type icons. The dev instance gets an isolated `AGENTMUX_HOME_OVERRIDE`.

## 12. Edge cases

- **No shell icon available** (a locked-down profile, helper cannot start): the glyph stays; one `warn` per session, no retry storm.
- **Two Filess, one folder:** one request per key (the routes and the page cache are shared).
- **Dark/light theme switch:** nothing to redo; icons are theme-independent. Switching `files:icons` swaps the render, no restart.
- **A file replaced with the same `mtime` and `len`:** keeps its old thumbnail (the same trade-off Explorer makes).
- **Very large images** (above the decoder limits): type icon, not a failure row.
- **Names with odd characters, long paths (`\\?\`), junctions:** `file-thumb` canonicalises; a junction is treated as the folder it is.
- **Hidden/system files:** shown with the same dimmed style as today; their icons are made the same way.

## 13. Decisions for the operator

Each with a recommendation; the rest of the spec assumes the recommendations.

1. **System icons on by default on Windows, with `files:icons = app` to keep today's glyphs?** Recommend **yes** on Windows; macOS and Linux switch to system once verified.
2. **Where do the shell calls run?** In a helper process (`agentmux-srv` re-executed with a subcommand), or in srv itself. Recommend **the helper**: one hung icon handler must not be able to stall every agent's terminal. If the subcommand dispatch turns out costly, a tiny second binary.
3. **Per-file icons (`.exe`, `.lnk`, `.ico`, custom folders) in the first release or later?** Recommend **P2, same release train as P1**: they are what Explorer parity looks like in Downloads and System32 and the helper already exists by then.
4. **Overlays.** Shortcut arrow: yes, as part of the `.lnk` icon (to be confirmed in the build). OneDrive/cloud and shield overlays: **not now**; later we could draw our own small badge from the file attributes the listing already reads, without per-file shell calls.
5. **Grid sizes (64 / 96 / 160 / 256) so thumbnails are large enough to be useful?** Recommend **yes**, grid only; the Details list stays 16 px icons (no thumbnails in rows).
6. **Thumbnail scope.** Windows: images and video from the shell; PDF and Office exactly when a handler is installed (nothing for us to configure); icon otherwise. Recommend **as stated**.
7. **Replace the page-side image decode with srv-side decode in P3?** Recommend **yes**: the page stops pulling up to 25 MB per photo into JavaScript.
8. **Video poster frames off Windows.** Recommend **icon only** for now; ask for a decoder (or an ffmpeg dependency, which is a supply-chain and size decision) only if the operator wants video thumbnails on macOS and Linux beyond what QuickLook and the freedesktop cache give.
9. **A disk thumbnail cache (256 MB, 30 days, clearable, `files:thumbnails = off` to disable)?** Recommend **yes**; Explorer keeps the same data already. Remote thumbnails: **memory only, on demand, images ≤ 2 MB**.
10. **New crates for macOS (`objc2-app-kit` family) and, optionally, Linux (`freedesktop-icons`)?** Recommend **allow the macOS family when P4 is built** (review the surface then) and **no Linux crate** (small hand-written index.theme reader).
