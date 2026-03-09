# Terminal Black Screen Race Condition — Deep Dive

**Date:** 2026-03-08
**Severity:** P1 (intermittent, no workaround except closing and reopening pane)
**Symptom:** Opening a terminal pane sometimes results in a black screen with a blinking cursor. Shell never loads. Pressing Enter/Space does nothing. Stuck forever.

---

## Executive Summary

There is a **subscribe-before-data race condition** in the terminal initialization path. The backend can spawn a PTY and emit shell output **before** the frontend has subscribed to receive it. Data that arrives during this window is either lost entirely or buffered in `heldData` and **never flushed** to xterm.js.

---

## The Race: Step by Step

### Frontend (term.tsx + termwrap.ts)

```
1. new TermWrap(blockId, ...)          // constructor, SYNC
   ├─ terminal = new Terminal(opts)
   ├─ terminal.open(connectElem)
   └─ handleResize()                   // SYNC, called from constructor
       └─ resyncController("initial resize")  // async, fire-and-forget
           └─ RPC: ControllerResyncCommand ──────► backend

2. fireAndForget(termWrap.initTerminal())  // async, NOT awaited
   ├─ getFileSubject(blockId, "term")      // creates Subject
   ├─ subject.subscribe(handleNewFileSubjectData)  // registers handler
   ├─ await loadInitialTerminalData()      // fetches cached + main file data
   └─ this.loaded = true                   // ◄── gate opens
```

### Backend (shell.rs)

```
3. Receives ControllerResyncCommand
   ├─ ShellController::new()
   ├─ ctrl.start()
   │   ├─ publish status = "RUNNING"
   │   ├─ openpty() with default 25x80
   │   ├─ spawn_command(shell)          // shell process starts
   │   └─ tokio::task::spawn_blocking   // PTY read loop starts IMMEDIATELY
   │       └─ loop { reader.read() → handle_append_block_file() → publish event }
   │
   └─ Shell emits prompt (e.g. "bash-5.1$ ")
      └─ PTY read loop captures it
         └─ Publishes blockfile event to frontend
```

### The Window

```
Timeline:
─────────────────────────────────────────────────────────────►

  handleResize()        initTerminal()         loaded=true
  sends ResyncCmd       subscribes to          gate opens
       │                file subject               │
       │                     │                     │
       ▼                     ▼                     ▼
  ─────┼─────────────────────┼─────────────────────┼──────
       │                     │                     │
       │    ┌────────────────┘                     │
       │    │  RACE WINDOW                         │
       │    │  Data arriving here is               │
       │    │  either LOST or BUFFERED             │
       │    │  in heldData (never flushed)         │
       │    └─────────────────────────────┐        │
       │                                  │        │
       ▼ backend spawns PTY              ▼        ▼
         shell emits prompt        subscribe   loaded=true
                                   registered
```

---

## Why Data Is Lost

### Path 1: Data arrives before `subscribe()` (line 596)

The global `blockfile` event handler in `global.ts:245` fires and calls `getFileSubject(blockId).next(data)`. But the Subject has no subscribers yet — `initTerminal()` hasn't reached line 596. **Data is silently dropped by the RxJS Subject.**

### Path 2: Data arrives after `subscribe()` but before `loaded = true` (line 600)

`handleNewFileSubjectData()` (line 652-667) buffers it:

```typescript
if (this.loaded) {
    this.doTerminalWrite(decodedData, null);  // write to xterm
} else {
    this.heldData.push(decodedData);          // buffer for later
}
```

**But `heldData` is never flushed.** `loadInitialTerminalData()` (line 687-718) writes cache data and main file data to xterm, sets `this.loaded = true`, but **never writes `this.heldData`**. The buffered data is permanently orphaned.

### Path 3: `loadInitialTerminalData()` fetches stale offset

Even if the HTTP fetch at line 711 grabs the main term file, the data may not include bytes that were appended *during* the fetch. The PTY read loop runs continuously — data appended between the file read and `loaded = true` goes to `heldData` and is never flushed.

---

## Why It's Intermittent

The race depends on timing:
- **Fast machine / slow shell**: `initTerminal()` completes before shell emits prompt → works fine
- **Slow machine / fast shell**: Shell prompt arrives during the race window → black screen
- **Shell integration scripts**: If bash/zsh sources `.bashrc`/`.zshrc` with slow plugins, the delay may push the prompt past the window → works
- **Simple shells**: `cmd.exe` or `pwsh` emit prompt instantly → more likely to hit the race

---

## Affected Code Locations

| File | Line | Issue |
|------|------|-------|
| `frontend/app/view/term/term.tsx` | 162 | `fireAndForget(initTerminal)` — not awaited |
| `frontend/app/view/term/termwrap.ts` | 749-751 | `resyncController()` called from `handleResize()` before subscribe |
| `frontend/app/view/term/termwrap.ts` | 595-596 | `subscribe()` happens after resync already sent |
| `frontend/app/view/term/termwrap.ts` | 600 | `this.loaded = true` set after `loadInitialTerminalData()` |
| `frontend/app/view/term/termwrap.ts` | 652-667 | `handleNewFileSubjectData()` buffers to `heldData` when `!loaded` |
| `frontend/app/view/term/termwrap.ts` | 687-718 | `loadInitialTerminalData()` — **never flushes `heldData`** |
| `agentmuxsrv-rs/src/backend/blockcontroller/shell.rs` | 522-544 | PTY read loop starts immediately, no frontend readiness check |

---

## Proposed Fix

### Minimal Fix: Flush `heldData` after loading

In `initTerminal()` (termwrap.ts), after `loadInitialTerminalData()` completes and `loaded` is set to true, flush any buffered data:

```typescript
async initTerminal() {
    // ... existing setup ...
    this.mainFileSubject = getFileSubject(this.blockId, TermFileName);
    this.mainFileSubject.subscribe(this.handleNewFileSubjectData.bind(this));
    try {
        await this.loadInitialTerminalData();
    } finally {
        this.loaded = true;
        // Flush any data that arrived during loading
        if (this.heldData.length > 0) {
            for (const data of this.heldData) {
                this.doTerminalWrite(data, null);
            }
            this.heldData = [];
        }
    }
    this.runProcessIdleTimeout();
}
```

This fixes **Path 2** (buffered but never flushed).

### Better Fix: Subscribe before resync

Move the file subject subscription into the constructor, **before** `handleResize()` triggers `resyncController()`:

```typescript
constructor(blockId, connectElem, opts, wrapOpts) {
    // ... existing setup ...
    this.mainFileSubject = getFileSubject(this.blockId, TermFileName);
    this.mainFileSubject.subscribe(this.handleNewFileSubjectData.bind(this));
    // NOW it's safe to trigger resync
    this.terminal.open(connectElem);
    this.handleResize();
}
```

This fixes **Path 1** (no subscriber) AND **Path 2** (with the heldData flush).

### Best Fix: Subscribe + flush + guarantee ordering

1. Subscribe to file subject in constructor (before resync)
2. Flush `heldData` after `loadInitialTerminalData()`
3. Move `resyncController()` into `initTerminal()` after subscribe, so the ordering is guaranteed:
   - subscribe → resync → load initial data → flush heldData → loaded = true

This eliminates the race entirely.

---

## Related: Input Blocked Too

Note that `handleTermData()` (line 629-640) also gates on `this.loaded`:

```typescript
handleTermData(data: string) {
    if (!this.loaded) {
        return;  // ◄── keyboard input silently dropped
    }
    // ...
}
```

This explains why pressing Enter/Space does nothing — input is dropped until `loaded = true`. But since `loaded` depends on `loadInitialTerminalData()` which does an HTTP fetch, and the fetch returns stale data (missing the prompt), the terminal appears stuck. The user types but nothing happens because `loaded` is true (fetch completed) but the prompt was lost.

Actually — `loaded` IS set to true after the fetch. So input *should* work after loading. The real issue is that the **prompt output was lost**, so the user sees a blank screen and thinks input isn't working, but the shell IS running and receiving keystrokes — it just never showed its prompt.

**Test:** If you hit this bug, try typing `echo hello` + Enter blindly. If "hello" appears, the shell is alive but the prompt was lost to the race.
