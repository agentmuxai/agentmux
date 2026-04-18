# SPEC: wave.ts Cleanup and Modularization

**Date:** 2026-04-17
**Status:** Draft

---

## What is wave.ts?

`frontend/wave.ts` is the **application initialization module** — the second
stage of bootstrap after `bootstrap.ts` calls `setupCefApi()`. It's named
"wave" because AgentMux was forked from WaveTerm. The name no longer means
anything.

**What it does (614 lines):**

1. **Host detection** — `isHostApp()`: checks if running inside the desktop app
2. **Init orchestration** — `initBare()` → `initHostWave()` or `initHostNewWindow()`:
   fetches client/workspace/tab data, creates layout, renders the app
3. **Instance tracking** — `initInstanceTracking()`: registers window count listeners
4. **App rendering** — `initWave()` → `render()`: mounts the SolidJS `<App>` component
5. **Workspace management** — `loadAllWorkspaceTabs()`, `reloadAllWorkspaceTabs()`
6. **Reinit on reconnect** — `reinitWave()`: re-fetches state after backend restart

**Dependencies:** Imports from 12+ modules (store, layout, services, RPC, utils).

---

## Problems

### 1. Name
"wave.ts" means nothing in AgentMux. New agents and contributors don't know
what it does without reading it. Should be `init.ts` or `app-init.ts`.

### 2. Size
614 lines doing 6 different things. Functions are tightly coupled — hard to
test any piece in isolation.

### 3. Testability
Zero tests. The init flow has complex branching (host vs non-host, main window
vs new window, first load vs reinit) with no coverage. Bugs like the Tauri
dead code survived for months because nothing exercised these paths.

### 4. Dead patterns
- `savedInitOpts` global mutable state
- `withTimeout` helper used in only one place
- `showStartupError` DOM manipulation that duplicates bootstrap.ts error display
- Comments referencing Tauri patterns that no longer exist

---

## Rename

| Old | New | Why |
|-----|-----|-----|
| `wave.ts` | `app-init.ts` | Describes what it does: initializes the app |

Also rename the export: `initBare()` → `initApp()`.

**Impact:** 2 files import from wave.ts:
- `bootstrap.ts` — `import { initBare } from "./wave"` → `import { initApp } from "./app-init"`
- `wave.ts` itself (internal calls)

Plus comments in a few files referencing "wave.ts" by name.

---

## Modularization Plan

Split into focused modules under `frontend/app/init/`:

```
frontend/app/init/
├── index.ts              # Re-exports initApp (entry point for bootstrap)
├── host-detect.ts        # isHostApp()
├── host-init.ts          # initHostWave(), initHostNewWindow()
├── instance-tracking.ts  # initInstanceTracking(), window count listeners
├── app-render.ts         # initWave(), reinitWave(), render(<App>)
├── workspace.ts          # loadAllWorkspaceTabs(), reloadAllWorkspaceTabs()
└── error-display.ts      # showStartupError()
```

### Module responsibilities

**`host-detect.ts`** (10 lines)
```typescript
export function isHostApp(): boolean {
    return typeof window.__AGENTMUX_IPC_PORT__ !== "undefined";
}
```
Testable: mock `window.__AGENTMUX_IPC_PORT__` in vitest.

**`host-init.ts`** (~120 lines)
- `initHostWave()` — main window init (fetch client, workspace, tab, etc.)
- `initHostNewWindow()` — secondary window init (get tear-off workspace or create new)
- Pure orchestration — calls services, no DOM manipulation

Testable: mock RPC services, verify correct call sequence.

**`instance-tracking.ts`** (~40 lines)
- `initInstanceTracking()` — subscribe to window count changes
- Isolated from rendering — just manages atoms

Testable: mock event subscriptions, verify atom updates.

**`app-render.ts`** (~80 lines)
- `initWave(initOpts)` — create layout, render `<App>`
- `reinitWave()` — re-fetch and re-render after backend restart

Testable: mock render, verify layout creation.

**`workspace.ts`** (~30 lines)
- `loadAllWorkspaceTabs(ws)` — resolve tab data for a workspace
- `reloadAllWorkspaceTabs(ws)` — re-resolve after changes

Testable: mock WOS, verify tab loading.

**`error-display.ts`** (~20 lines)
- `showStartupError(message)` — DOM error overlay

Testable: call and check DOM output.

**`index.ts`** (~30 lines)
- `initApp()` — the main entry point, replaces `initBare()`
- Wires together host-detect → host-init → app-render
- Exported for bootstrap.ts

---

## Test Plan

### Unit tests (vitest)

| Module | Tests | What |
|--------|-------|------|
| `host-detect` | 2 | Returns true/false based on window global |
| `workspace` | 3 | Tab loading, empty workspace, missing tabs |
| `error-display` | 2 | Error shown in DOM, clears previous content |

### Integration tests (vitest + jsdom)

| Module | Tests | What |
|--------|-------|------|
| `host-init` | 4 | Main window init sequence, new window init, error handling, tear-off workspace |
| `app-render` | 3 | First render, reinit, layout creation |
| `instance-tracking` | 2 | Window count subscribe, unsubscribe on cleanup |

### What we CAN'T test

- Actual CEF API availability (requires running inside the desktop app)
- Real WebSocket connection (needs backend sidecar)
- These stay as manual smoke tests during `task dev` / portable testing

---

## Implementation Plan

### PR 1: Rename wave.ts → app-init.ts

1. `git mv frontend/wave.ts frontend/app-init.ts`
2. Update `bootstrap.ts` import
3. Rename `initBare()` → `initApp()`
4. Update all comments referencing "wave.ts"
5. Update `CLAUDE.md` if it references wave.ts

**Effort:** 30 minutes. Zero logic changes.

### PR 2: Extract host-detect + error-display + workspace

1. Create `frontend/app/init/` directory
2. Move pure functions out of app-init.ts
3. Add unit tests for each extracted module
4. app-init.ts imports from the new modules

**Effort:** 1-2 hours.

### PR 3: Extract host-init + app-render + instance-tracking

1. Move the larger orchestration functions
2. Add integration tests with mocked services
3. app-init.ts becomes a thin orchestrator (~30 lines)

**Effort:** 2-3 hours.

### PR 4: Delete app-init.ts, replace with init/index.ts

1. Final rename: `frontend/app-init.ts` → `frontend/app/init/index.ts`
2. bootstrap.ts imports from `@/app/init`
3. Clean, modular, tested

**Effort:** 30 minutes.

---

## Non-Goals

- **Changing the init logic.** This is a pure refactor — same behavior,
  same call order, just better organized and tested.
- **Removing @tauri-apps packages.** That's a separate cleanup tracked in
  the audit report.
- **Renaming the `agentmux-cef` crate.** That's a Rust binary name, not
  related to this frontend cleanup.
