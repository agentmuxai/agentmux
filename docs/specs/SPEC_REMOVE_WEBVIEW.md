# Spec: Remove Built-in Browser (Webview) and Tsunami

**Date:** 2026-03-05
**Author:** Agent2
**Branch:** agent2/remove-monaco-editor
**Status:** Pre-implementation

---

## Motivation

AgentMux ships a full built-in web browser (URL bar, back/forward, bookmarks, search, zoom, mobile UA emulation, DevTools, cookie management). Users don't need it -- they use their own window manager and system browser. Tsunami (a shell controller UI) extends the webview and is also unused in practice.

The "web" view type is not even registered in the BlockRegistry, meaning it's inaccessible through normal block creation. Tsunami IS registered but depends entirely on WebViewModel.

## Scope

Remove the built-in browser and Tsunami view, plus all orphaned types, settings, and API stubs.

## Files to Delete

| File | Lines | Description |
|------|-------|-------------|
| `frontend/app/view/webview/webview.tsx` | 1,077 | Full browser implementation (WebViewModel + WebView component) |
| `frontend/app/view/webview/webview.scss` | 52 | Browser styling |
| `frontend/app/view/tsunami/tsunami.tsx` | 250 | Shell controller UI (extends WebViewModel) |

**Total removed:** ~1,379 lines

## Files to Edit

### `frontend/app/block/block.tsx`

Remove:
- Line 15: `import { TsunamiViewModel } from "@/app/view/tsunami/tsunami";`
- Line 45: `BlockRegistry.set("tsunami", TsunamiViewModel);`

### `frontend/types/custom.d.ts`

Remove 4 webview API type declarations:
- `getWebviewPreload: () => string;`
- `setWebviewFocus: (focusedId: number) => void;`
- `registerGlobalWebviewKeys: (keys: string[]) => void;`
- `clearWebviewStorage: (webContentsId: number) => Promise<void>;`

### `frontend/util/tauri-api.ts`

Remove corresponding stub implementations:
- `getWebviewPreload: () => ""`
- `setWebviewFocus: (focusedId: number) => { ... }`
- `registerGlobalWebviewKeys: (keys: string[]) => { ... }`

### `frontend/types/gotypes.d.ts`

Remove orphaned setting/metadata keys:
- `web:zoom`, `web:hidenav`, `web:partition`, `web:useragenttype` (metadata)
- `web:openlinksinternally`, `web:defaulturl`, `web:defaultsearch` (settings)
- `tsunami:*`, `tsunami:sdkreplacepath`, `tsunami:apppath`, `tsunami:scaffoldpath`, `tsunami:env` (metadata)
- `tsunami:title`, `tsunami:shortdesc`, `tsunami:schemas` (runtime info)

### `frontend/app/store/global.ts`

Remove `web:openlinksinternally` reference in the openLink function.

## Files Left Unchanged (Safe)

| File | Reason |
|------|--------|
| `frontend/app/block/autotitle.ts:176` | `case "tsunami": return "Tsunami"` -- harmless fallback string for any legacy blocks |
| `frontend/app/view/helpview/helpview.tsx` | Comment-only reference ("Previously extended WebViewModel...") |
| `frontend/app/block/autotitle.ts:166` | `case "preview": return generatePreviewTitle(block)` -- independent of webview code |

## Libraries Removed

| Package | Size | Notes |
|---------|------|-------|
| `react-zoom-pan-pinch` | ~200 KB | Confirmed unused anywhere in codebase |

## Libraries NOT Affected

| Package | Used By | Confirmed Independent |
|---------|---------|----------------------|
| `html-to-image` | `TileLayout.tsx` drag-drop previews | Yes |
| `parse-srcset` | `markdown-util.ts` image resolution | Yes |

## Estimated Savings

- ~1,379 lines of code removed
- ~200 KB from react-zoom-pan-pinch removal
- Reduced attack surface (no iframe-based browser)
- Reduced complexity for maintenance

## Testing

- [ ] `npx vite build --config vite.config.tauri.ts` succeeds
- [ ] No console errors referencing webview or tsunami
- [ ] All other block types still render (term, agent, help, sysinfo, vdom, launcher)
- [ ] `npm test` passes (no webview/tsunami test cases exist)
- [ ] helpview still works (already decoupled from WebViewModel)
