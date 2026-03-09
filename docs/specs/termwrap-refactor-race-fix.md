# Spec: TermWrap Refactor — Fix Terminal Init Race Condition

**Date:** 2026-03-08
**Status:** Proposed
**Related:** `docs/investigations/terminal-black-screen-race-condition.md`

---

## Problem

Opening a terminal pane sometimes shows a permanent black screen with cursor. The shell is running but its initial output (prompt) was lost to a race condition between backend PTY spawn and frontend data subscription.

Root cause: the constructor triggers `handleResize()` → `resyncController()` (which spawns the PTY on the backend), but data subscription happens later in `initTerminal()` (called via `fireAndForget`). Data arriving in between is either dropped (no subscriber) or buffered in `heldData` and never flushed.

---

## Current Architecture (Broken)

```
termwrap.ts (781 lines, single file)
├── Module-level functions (lines 1-443)
│   ├── detectWebGLSupport()
│   ├── registerAgent() / unregisterAgent() / handleAgentIdChange()
│   ├── handleOscWaveCommand()      — OSC 9283
│   ├── handleOsc7Command()         — OSC 7 (cwd)
│   ├── handleOscTitleCommand()     — OSC 0/2 (window title)
│   └── handleOsc16162Command()     — OSC 16162 (shell integration)
│
└── class TermWrap (lines 445-781)
    ├── constructor()               — 107 lines, does TOO MUCH:
    │   ├── Create Terminal + load 7 addons
    │   ├── Register 5 OSC handlers
    │   ├── terminal.open(elem)
    │   ├── handleResize() ─── triggers resyncController() ─── spawns PTY
    │   └── Paste handler setup
    │
    ├── initTerminal()              — called LATER via fireAndForget
    │   ├── Register onData/onKey/onSelection handlers
    │   ├── Subscribe to file subject     ◄── TOO LATE, PTY already running
    │   ├── loadInitialTerminalData()
    │   └── this.loaded = true            ◄── heldData never flushed
    │
    ├── Data handling
    │   ├── handleNewFileSubjectData()
    │   ├── doTerminalWrite()
    │   └── loadInitialTerminalData()
    │
    ├── Lifecycle
    │   ├── dispose()
    │   ├── handleResize()
    │   └── resyncController()
    │
    └── Caching
        ├── processAndCacheData()
        └── runProcessIdleTimeout()
```

### Problems with Current Structure

1. **Constructor does too much** — 107 lines mixing terminal setup, addon loading, OSC registration, DOM attachment, resize handling, AND triggering backend shell spawn
2. **Two-phase init is broken** — constructor triggers resync (spawns PTY), but `initTerminal()` subscribes to data later. The gap is the race window.
3. **`heldData` never flushed** — data buffered during loading is permanently lost
4. **OSC handlers are 250 lines of module-level functions** — clutters the file, separate concern from terminal I/O
5. **Agent registration mixed in** — reactive agent registration is unrelated to terminal rendering
6. **No clear lifecycle phases** — impossible to reason about initialization order

---

## Proposed Architecture

Split into 4 files with a strict initialization sequence that eliminates the race:

### File Structure

```
frontend/app/view/term/
├── termwrap.ts          — TermWrap class (slim: lifecycle + I/O + resize)
├── termosc.ts           — OSC handler functions (extracted, unchanged logic)
├── termagent.ts         — Agent registration helpers (extracted, unchanged logic)
└── (existing files unchanged)
```

### New TermWrap Lifecycle — 3 Phases

```
Phase 1: CONSTRUCT (sync)
  ├── Create Terminal instance
  ├── Load addons (webgl, fit, serialize, search, unicode, weblinks, filelinks)
  ├── Register OSC handlers
  ├── Attach key event handler
  └── Store references (blockId, connectElem, sendDataHandler)

  *** NO resize, NO open, NO resync, NO subscription ***

Phase 2: INIT (async, called explicitly)
  ├── terminal.open(connectElem)           — mount to DOM
  ├── Subscribe to file subject            — BEFORE any backend communication
  ├── Register onData/onKey/onSelection    — input handlers
  ├── fitAddon.fit()                       — get actual terminal size
  ├── loadInitialTerminalData()            — fetch cached + current data
  ├── Flush heldData                       — write any buffered data to xterm
  ├── this.loaded = true                   — open the gate
  ├── resyncController("initial")          — NOW tell backend to start PTY
  └── Start idle cache timeout

  *** Subscribe FIRST, resync LAST ***

Phase 3: RUNNING
  ├── handleResize() — fit + send size to backend (no resync, already running)
  ├── handleNewFileSubjectData() — write to xterm (loaded=true, no buffering)
  └── processAndCacheData() — periodic serialization
```

### Key Changes

#### 1. Subscribe before resync (eliminates the race)

```typescript
// OLD (broken): constructor resync → later subscribe
constructor() {
    // ... setup ...
    this.terminal.open(connectElem);
    this.handleResize();  // → resyncController() → spawns PTY
}
async initTerminal() {
    this.mainFileSubject = getFileSubject(this.blockId, TermFileName);
    this.mainFileSubject.subscribe(...);  // TOO LATE
}

// NEW (fixed): subscribe → load → resync
async init() {
    this.terminal.open(this.connectElem);
    // Subscribe FIRST
    this.mainFileSubject = getFileSubject(this.blockId, TermFileName);
    this.mainFileSubject.subscribe(this.handleNewFileSubjectData.bind(this));
    // Load existing data
    await this.loadInitialTerminalData();
    // Flush anything that arrived during loading
    this.flushHeldData();
    this.loaded = true;
    // NOW tell backend to start (or resync) the shell
    this.fitAddon.fit();
    await this.resyncController("init");
    this.runProcessIdleTimeout();
}
```

#### 2. Flush `heldData` (fixes data loss)

```typescript
private flushHeldData(): void {
    for (const data of this.heldData) {
        this.doTerminalWrite(data, null);
    }
    this.heldData = [];
}
```

#### 3. Separate resize from initial resync

```typescript
// OLD: handleResize() triggers resync on first call
handleResize() {
    this.fitAddon.fit();
    // ... send size ...
    if (!this.hasResized) {
        this.hasResized = true;
        this.resyncController("initial resize");  // side effect!
    }
}

// NEW: handleResize() ONLY handles resize, init() handles first resync
handleResize() {
    const oldRows = this.terminal.rows;
    const oldCols = this.terminal.cols;
    this.fitAddon.fit();
    if (oldRows !== this.terminal.rows || oldCols !== this.terminal.cols) {
        this.sendTermSize();
    }
}
```

#### 4. Consumer change in term.tsx

```typescript
// OLD
const termWrap = new TermWrap(blockId, connectElemRef.current, opts, waveOpts);
// ... setup ResizeObserver, onSearchResultsDidChange ...
fireAndForget(termWrap.initTerminal.bind(termWrap));

// NEW
const termWrap = new TermWrap(blockId, connectElemRef.current, opts, waveOpts);
// ... setup ResizeObserver, onSearchResultsDidChange ...
fireAndForget(async () => {
    await termWrap.init();
});
```

`fireAndForget` is still used (React effect can't be async), but the ordering inside `init()` is now correct.

---

## Extract: termosc.ts

Move OSC handler functions out of termwrap.ts. These are pure functions that take `(data, blockId, loaded)` and return `boolean`. No change to logic.

```typescript
// termosc.ts — Terminal OSC escape sequence handlers
export function handleOscWaveCommand(data: string, blockId: string, loaded: boolean): boolean { ... }
export function handleOsc7Command(data: string, blockId: string, loaded: boolean): boolean { ... }
export function handleOscTitleCommand(data: string, blockId: string, loaded: boolean): boolean { ... }
export function handleOsc16162Command(data: string, blockId: string, loaded: boolean, terminal: Terminal): boolean { ... }
```

~200 lines extracted from termwrap.ts.

## Extract: termagent.ts

Move agent registration functions. These are standalone async helpers with no TermWrap dependency.

```typescript
// termagent.ts — Reactive agent registration
export const registeredAgentsByBlock: Map<string, string>;
export async function registerAgent(agentId: string, blockId: string, tabId?: string): Promise<void> { ... }
export async function unregisterAgent(agentId: string): Promise<void> { ... }
export function handleAgentIdChange(blockId: string, newAgentId: string | undefined, tabId?: string): void { ... }
```

~70 lines extracted from termwrap.ts.

---

## Result

### Before
- `termwrap.ts`: 781 lines, one file, tangled init
- Race condition in init ordering
- `heldData` never flushed

### After
- `termwrap.ts`: ~430 lines (TermWrap class + constants + types)
- `termosc.ts`: ~220 lines (OSC handlers)
- `termagent.ts`: ~80 lines (agent registration)
- **Strict init ordering: subscribe → load → flush → resync**
- **Race condition eliminated by construction**

### What Does NOT Change
- All OSC handler logic (just moved to new file)
- Agent registration logic (just moved to new file)
- xterm.js addon loading (same addons, same order)
- `loadInitialTerminalData()` logic (cache + main file fetch)
- `doTerminalWrite()` / `processAndCacheData()` / `runProcessIdleTimeout()`
- `term.tsx` React component structure (only the `fireAndForget` call changes)
- Backend shell.rs (no backend changes needed)

---

## Test Plan

- [ ] Open terminal pane — shell prompt appears reliably
- [ ] Rapidly open 5+ terminal panes — all show prompts (stress test the race)
- [ ] Split pane horizontally/vertically — new terminals load correctly
- [ ] Close and reopen terminal — cached data restored
- [ ] SSH terminal — shell loads on remote
- [ ] OSC 7 (cwd tracking) — still works after extract
- [ ] OSC 16162 (shell integration) — pane title/color still update
- [ ] Agent registration — AGENTMUX_AGENT_ID still registers with reactive backend
- [ ] Resize terminal — size updates sent to backend
- [ ] Search (Ctrl+F) — still works
- [ ] WebGL/Canvas renderer — still loads with fallback
- [ ] Settings change (font size, theme) — terminal recreates correctly
