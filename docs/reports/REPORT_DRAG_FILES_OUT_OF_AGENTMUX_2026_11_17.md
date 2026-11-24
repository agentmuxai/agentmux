# REPORT: dragging files (images, media) out of AgentMux to the desktop and other apps

Research for Kanban #47953. Build follows in #47954 after approval. Nothing here is built; no lab was run.

**The ask, restated as a requirement.** A person should be able to take an image or other media file from an Editor tab, an image view or a row in the Files pane and drop it on the desktop, a file manager, or another app such as a chat client or a browser page, on Windows, macOS and Linux, and have the other app receive the real file.

Evidence marks: **[src]** read in the Chromium 154 source tree the CEF fork is built from, **[repo]** read in this repository, **[doc]** from vendor documentation or an upstream project, **[?]** inferred, needs the spike in section 6 before anyone relies on it.

## 1. Summary

1. **What happens today.** A Files row drags paths inside the app and puts the paths as *text* on the drag ([repo] `frontend/app/drag/file-drop.ts` `beginPathDrag`). Dropped on a chat client, the person gets the path as text, not the file. The image viewer disables dragging ([repo] `image-viewer-parts.tsx:69-70`). A document tab drags within the Editor's tab strips (reorder, move to another pane) and never tears off ([repo] `pane-tab-pill.tsx:188`), so a release outside the window does nothing today.
2. **CEF has no drag-source API for our window type.** `StartDragging` and `DragSourceEndedAt` exist only when the browser draws itself off-screen; the only drag handler we have is `OnDragEnter`, for incoming drags ([src] `cef/include/cef_browser.h`, `cef_drag_handler.h`; [repo] `crates/cef/src/client/handlers.rs`). A web page's own drag is turned into the operating system's drag by Chromium. So the question is how to get real file paths into *that* drag.
3. **Chromium already carries real files out on Windows and Linux** when the drag holds a `File` that has a path on disk: it fills `DropData::filenames`, which `PrepareDragData` puts on the OS drag as `CF_HDROP` (Windows) or file URIs (Linux). The renderer must be allowed to read the file ([src] `web_contents_view_aura.cc:295`, `render_widget_host_impl.cc:3208-3213`). **Chromium on macOS does not**: it writes file *promises* and image bytes, never file URLs for `filenames` ([src] `web_drag_source_mac.mm`).
4. **We already own the two native hook points** that matter: a swap of libcef's `DoDragDrop` on Windows ([repo] `crates/cef/src/commands/drop_source_wrap.rs`) and a swizzle of `-[NSView beginDraggingSessionWithItems:event:source:]` on macOS ([repo] `crates/cef/src/macos_compat.rs:155`).
5. **Recommendation.** One frontend gesture (a normal HTML5 drag with an extra, nonce-carrying marker), and per platform the smallest thing that turns the marker into real files: Windows and Linux through Chromium's own `filenames` path (no native drag code), macOS through the existing swizzle adding a file URL to the drag item. First step: **Windows, local files, from a Files row and the image viewer**, because it needs no native code if the spike in section 6 confirms the file-access grant; the fallback is a wrapped data object in the `DoDragDrop` swap we already have. Then macOS, then Linux, then the Editor tab and remote files.

## 2. What the drop targets accept

| Target | Windows | macOS | Linux (X11, Wayland) |
|---|---|---|---|
| File manager, desktop | `CF_HDROP` (real files; the effect says copy, move or link). Data that is not a file on disk uses `FileGroupDescriptorW` + `FileContents` (virtual files, rendered when dropped) [doc] | File URL (`public.file-url`) for a file that exists; a file promise for one that does not exist yet (`NSFilePromiseProvider`, or the legacy promise types Chromium writes) [doc][src] | `text/uri-list` of `file://` URIs, on X11 (XDND) and on Wayland (`wl_data_device`) [?] standard; Chromium's Wayland provider offers `text/uri-list` when the drag holds a file or URL and builds it from them [src] `wayland_exchange_data_provider.cc:225-330` |
| Chat client, browser page (Discord, Slack, Chrome) | Chromium-based, so the same path as Chrome: it reads `CF_HDROP`, and on Windows also virtual files [src] `web_contents_view_aura.cc:748-760` | Chromium-based targets take file URLs; promises too [?] | Same as Chrome on Linux: `text/uri-list` files [?] |
| Real file on disk | `CF_HDROP` | file URL | `text/uri-list` |
| Only in memory (image bytes, a remote file) | virtual file (`FileContents`), or write a temp file and send it as a real one | file promise, or a temp file as a real one | `application/octet-stream;name=...` is offered by Chromium for file contents [src] but few targets read it: write a temp file and send it as a real one |

Notes:
- **A real file is accepted everywhere.** Memory-only data is accepted well only on Windows and macOS. So the simplest correct design makes a real file (a temp copy when needed) and drags that.
- **Linux sandboxes.** Chromium's Wayland provider also has the portal file-transfer types, for targets running in a Flatpak or Snap sandbox [src] (`kMimeTypePortalFileTransfer`). Worth a check on a real desktop; not needed for the first step.
- **Chromium 108-era reports** that the Wayland provider could not extract `text/uri-list` are not true of the 154 source: `ExtractData` handles it ([src] line 310).

## 3. How our stack can start the drag

### 3.1 What Chromium does with a page's drag [src]

`RenderWidgetHostImpl::StartDragging` receives the page's `DataTransfer` as `DropData`, filters it, and hands it to the view, which fills the OS drag:

| In the page's drag | Windows | macOS | Linux |
|---|---|---|---|
| `DownloadURL` (`mime:name:url`) | Chromium downloads the URL into a temp directory when dropped, and gives the target a `CF_HDROP` of it (temp file gets the Mark of the Web; deleted at reboot) | file promise, written when dropped | **not supported** (the download branch is `#if BUILDFLAG(IS_WIN)`) |
| a `File` with a path on disk (`filenames`) | `CF_HDROP` | **not written** | file URIs |
| image bytes (`file_contents`), from an `<img>` drag | virtual file | promise plus the bytes | `application/octet-stream;name=` |
| JS-made `File` (bytes) | **behind a test-only Blink flag** (`DragAndDropJSFileObjects`, status `test`), images only, validated by magic bytes | same flag | same flag |
| `text/uri-list` with `file://` | **stripped** for a renderer that may not request `file:` URLs: `FilterURL` replaces any URL the process `CanRequestURL` refuses (`render_process_host_impl.cc:4889-4917`) [src]; that our UI renderers lack `file:` access is [?] and part of the spike | same | same |

Consequences:
- A `file://` URI in the drag cannot work; the paths must come through `filenames` or a native hook.
- `DownloadURL` works on Windows and macOS but needs an http(s) URL the browser's own network stack can fetch without our custom headers, so srv would need a short-lived signed GET route for files. One file per drag. Windows adds the Mark of the Web. Nothing on Linux.
- The JS-made-`File` route exists but is a test flag and images only. Not a base to build on.
- `filenames` reach the OS drag only for paths the renderer may read. Chromium grants that for files the person dropped or chose in a file input, and for the DevTools command `DOM.setFileInputFiles`, which calls `GrantReadFile` for the renderer ([src] `devtools/protocol/dom_handler.cc:42-56`) **provided the session is allowed file access** (`allow_file_access_`). Whether our own CDP sessions are is the first thing the spike checks [?].

### 3.2 The hook points we already own [repo]

- **Windows.** libcef reaches `DoDragDrop` through its delay-load table; we replace the entry with a function that passes a wrapping `IDropSource` (for the drag cursor, `drop_source_wrap.rs`). The same function also receives the drag's `IDataObject`, so a wrapped data object that also serves `CF_HDROP` is a natural extension. It needs the full `IDataObject` surface wrapped (today only `GetData` is intercepted).
- **macOS.** Chromium starts every drag with `beginDraggingSessionWithItems:event:source:`; we already swizzle it. The item's writer is an `NSPasteboardItem` with a lazy data provider [?]; adding a `public.file-url` flavor to it (and further items for further files) makes one drag that is a file to Finder and Discord and still carries Chromium's own types for drops inside the app.
- **Linux.** No equivalent hook. Chromium's Aura/Ozone view builds the data, so either the `filenames` route above (no patch), or a small patch in the CEF fork (`PrepareDragData` adds `SetFilenames` when the drag's marker names files in an AgentMux registry). The patch means a new CEF release for three platforms; avoid it unless the `filenames` route fails.

### 3.3 How others do it [doc]

- **Electron** `webContents.startDrag({file|files, icon})`: Windows and Linux build an `OSExchangeData` with `SetFilenames` and start Chromium's own drag (copy and link allowed); macOS puts `NSURL`s on `NSDraggingItem`s and begins a dragging session with a synthesized mouse event. It writes file URLs, **not** file promises. Known quirks: the cursor always shows "copy", multiple files are limited on Windows, and on Linux the call blocks until the drop. (electron/electron `drag_util_views.cc`, `drag_util_mac.mm`, PR #6333, issues #45197, #15361.)
- **VS Code** dragged an editor tab to the file explorer with `DownloadURL`; that broke when file access was restricted (issue #128975), and a later change enforces file URIs for it. I could not confirm what it does today beyond that.
- **Windows shell** offers `SHDoDragDrop`, which makes the drag image from the data object or the window, and a drop source of its own (Vista and later). Useful if we ever start a native drag ourselves instead of wrapping Chromium's.

Electron's approach (a native "start drag with these files" call) is the cleanest in principle but needs access to Chromium's internal drag client, which the CEF C API does not give us for our window type. Our equivalent is the marker plus the hooks above.

## 4. Where the drag starts

| Source | Today | Needed |
|---|---|---|
| Files row | HTML5 drag (`beginPathDrag`); refused for a remote or confined folder (`files-view-drag.ts:66`) | add the file drag marker for local rows; remote rows get a temp copy (below) |
| Image or media view in the Editor | `draggable={false}` and `preventDefault` on `dragstart` | make the image draggable with the file marker and a thumbnail as the drag image |
| Document tab | pragmatic-dnd drag for reorder and move between panes, "never a tear-off" (`pane-tab-pill.tsx:188-221`); its data is in-app only | add the file marker for a tab that shows a file on this computer (an unsaved scratch tab has no file and is not draggable out). A release outside every window then means "drop this file there", and nothing else claims that release. Pane tabs (agent, terminal) do tear off and get no file marker. |

**Remote files (SSH, WSL, container).** The bytes are not on this computer. At `dragstart` the frontend can't wait for a copy, so:
1. On pointer-down on a remote row or tab, start a background copy to `<mux data dir>/tmp/drag-out/<random>/<name>` (same read path the Editor uses to open the file), capped at a size (say 200 MB), with a progress hint on the row.
2. If the drag starts before the copy is done, refuse it with a toast naming the file ("still copying"); never start a drag that holds nothing.
3. Clean up on `dragend` after a delay (the target may still be reading; Chromium on Windows leaves its temp files for the next reboot for the same reason), and sweep the folder at startup and hourly, deleting anything older than an hour. The existing screenshot folder already prunes by age the same way ([repo] `ui_handlers.rs` `prune_old_screenshots`).
4. A promise or virtual file (copy only when dropped) is the better end state on Windows and macOS, and is later work.

## 5. Security

- **Only a gesture of the person, only files they can see.** The marker is set by the app's own UI in a trusted `dragstart` (`event.isTrusted`), from rows or documents the person sees and picked. No agent action starts one; an agent can't make a pointer gesture.
- **A nonce, not paths, in the drag.** Before the drag, the UI asks the host to register the paths (an IPC only app windows have, not Browser panes) and gets a single-use nonce valid for about 10 seconds. The drag carries only the nonce. The hook honours a nonce it issued, once; anything else is ignored. A third-party page in a Browser pane can't register paths, so it can't make AgentMux attach local files to its own drag.
- **Source check as a second lock.** The renderer-initiated tag on the drag (Chromium records the origin of the page that started it) must be the app's own origin; a drag from any other origin is never given files.
- **Path rules.** The host re-checks each registered path: a file (not a directory, for the first step), readable, under the limit, not inside another agent's workspace unless it is what the person selected in their own view. A confined Files pane (`model.root()`) keeps handing out nothing.
- **Temp copies** live in a folder only the host writes, are named by the host (no path from the page), and are swept as in section 4.
- **No credentials** are involved; the vault's files are never draggable.

## 6. Recommendation, first step and test plan

### Design per platform

| Platform | Route | Native code |
|---|---|---|
| Windows | the page drags a `File` that has a path (the host grants read access through CDP `DOM.setFileInputFiles` on an input the page made), so Chromium's `SetFilenames` gives `CF_HDROP`. Fallback if file access is not allowed: wrap the data object in the `DoDragDrop` swap and add `CF_HDROP` for a valid nonce. | none, or about 250 lines (wrapped `IDataObject`) |
| macOS | swizzle: for a valid nonce, add `public.file-url` to the drag item (one extra item per further file) | about 80 lines in `macos_compat.rs` |
| Linux | same page-side route as Windows (`filenames` become file URIs); a CEF patch only if that fails | none, or a fork patch |

All three keep Chromium's drag image, cursor and in-app drop behaviour, so dropping a row inside AgentMux (composer, Editor, Files) is unchanged.

### First step (small)

Windows, local files, from a Files row and from the image viewer, one or several files, to Explorer, the desktop, and a Chromium-based chat client and browser page. No tab drag, no remote files, no macOS or Linux. This also proves the browser-side code that Linux shares.

### Spike before building (needs no real mouse)

1. In my own task dev build, over CDP: does `DOM.setFileInputFiles` work in our session (it refuses when file access is not allowed)? Then does `input.files[0]` carry a path, and does a synthesized drag (`Input.setInterceptDrags` + `Input.dispatchMouseEvent`, which reports the drag's `files` in `Input.dragIntercepted` without a real drag) include it after Chromium's filtering? This answers the Windows and Linux route without touching the screen.
2. On a macOS tester host, with a small test build: does adding `public.file-url` to the swizzled item's `NSPasteboardItem` make Finder and a Chromium app take the file, and do in-app drops still work?

### Test plan for the drop (each needs the operator's OK, as a real-mouse run)

| OS | Targets | Check |
|---|---|---|
| Windows | Explorer window, desktop, a Chromium-based chat client, Chrome on a page with a file drop zone, a mail client | the file arrives with its name and content; several files arrive together; copy versus move (Shift/Ctrl) behaves; the source file is untouched; dropping inside AgentMux (composer, Editor, Files) still works; cancel with Esc leaves no temp files |
| macOS (tester host) | Finder, desktop, a chat client, Chrome, Mail | as above; look check of the drag image and cursor; the Dock and Spaces behave |
| Linux (tester host) | the file manager of GNOME (X11 and Wayland) and of KDE, Chrome, a chat client | as above, on both display servers; a Flatpak or Snap target (portal case) |

Look and behaviour checks follow the visual-change rule: macOS and Linux testers verify before merge.

## 7. Open questions for the operator

1. **Dirty tabs.** A tab with unsaved changes would drag the file as saved on disk, not the buffer. Drag the saved file with a note, or refuse until saved?
2. **Remote files.** Copy on pointer-down with a size cap (200 MB), refusing the drag until it is done: OK for the first remote version?
3. **Directories and several files.** First step is files only; folders and multi-select from the Files pane right after: OK?
4. **Mark of the Web** (only if `DownloadURL` is used as a stop-gap on Windows): acceptable for images, not for executables. The plan avoids `DownloadURL`, so this only matters if the file-access spike fails.

## 8. Sources

- Chromium 154 source (the tree the CEF fork is built from): `content/browser/web_contents/web_contents_view_aura.cc`, `content/app_shim_remote_cocoa/web_drag_source_mac.mm`, `content/browser/renderer_host/render_widget_host_impl.cc` (`StartDragging`), `ui/ozone/platform/wayland/host/wayland_exchange_data_provider.cc`, `ui/base/dragdrop/os_exchange_data_provider_win.cc`, `third_party/blink/renderer/core/clipboard/data_object.cc`, `content/browser/devtools/protocol/dom_handler.cc`, CEF `include/cef_browser.h` and `cef_drag_handler.h`.
- Electron: [PR #6333](https://github.com/electron/electron/pull/6333), [issue #45197](https://github.com/electron/electron/issues/45197), [issue #15361](https://github.com/electron/electron/issues/15361), `shell/browser/ui/drag_util_views.cc`, `drag_util_mac.mm`.
- VS Code: [issue #128975](https://github.com/microsoft/vscode/issues/128975).
- Chromium `DownloadURL`: [web.dev case study](https://web.dev/case-studies/box-dnd-download), [Chromium issue 40443042](https://issues.chromium.org/issues/40443042).
- Wayland drag support history: [Chromium issue 40589809](https://issues.chromium.org/issues/40589809).
- Microsoft: [SHDoDragDrop](https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/nf-shlobj_core-shdodragdrop).
- Apple: [NSFilePromiseProvider](https://developer.apple.com/documentation/appkit/nsfilepromiseprovider).
- Repository: `frontend/app/drag/file-drop.ts`, `frontend/app/view/files/files-view-drag.ts`, `frontend/app/view/editor/viewers/image/image-viewer-parts.tsx`, `crates/cef/src/commands/drop_source_wrap.rs`, `crates/cef/src/macos_compat.rs`, `crates/cef/src/drag_stash.rs`, `crates/cef/src/client/handlers.rs`, `docs/specs/SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md`.
