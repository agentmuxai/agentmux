# SPEC: Terminal scrollback doesn't survive reconnect (all `view:"term"` panes)

**Date:** 2026-07-23
**Status:** implemented — PR #2279; verified in code 2026-08-10.
**Severity:** Medium (no data loss beyond the session — output is gone, not corrupted — but a real, visible regression in usability for both the agent-shell drawer and standalone Terminal panes)
**Trigger:** Reported while testing PR #2278 (shell-drawer log-panel removal) — the report ("close and reopen the shell, prior content is gone, resets every time") turned out to be a pre-existing gap unrelated to that PR's scope, discovered during live verification.

---

## 0. Ask

> "when i close and reopen and shell, can I see the prior? it appears to reset everytime, try again"

This document is the analysis + design for the underlying persistence gap, written before implementation per the user's request ("write the spec to file") rather than picking one option and shipping it blind, since the fix touches shared backend infrastructure used by every terminal in the app, not just the agent-shell drawer PR that surfaced it.

---

## 1. What's actually happening (confirmed, not hypothesized)

**Not specific to `AgentShellSubblock` or PR #2278.** Both the agent-shell drawer's terminal and a plain standalone Terminal-view block go through the exact same `ShellController` (`agentmux-srv/src/backend/blockcontroller/mod.rs:423-437` — one factory, shared by `BLOCK_CONTROLLER_SHELL`/`BLOCK_CONTROLLER_CMD`, used by both `view:"term"` surfaces) and the exact same frontend `TermWrap` (`frontend/app/view/term/termwrap.ts`, used identically by `AgentShellSubblock.tsx:131-157` and the main `term.tsx:171-202`). A standalone Terminal pane whose tab gets torn down and rebuilt loses scrollback in the identical way — this was just never noticed as sharply as it is now that the agent-shell drawer's mount/unmount cycle (drawer close/reopen, `<Show>` in `agent-view.tsx`) makes it happen constantly during normal use.

### 1.1 The frontend already has a working restore mechanism — it just never gets real data back

`TermWrap.init()` (`termwrap.ts:263-275`) subscribes to live output *first*, then explicitly fetches durable history before considering itself "loaded":

```ts
this.mainFileSubject = getFileSubject(this.blockId, TermFileName);
this.mainFileSubject.subscribe(this.handleNewFileSubjectData.bind(this));
try {
    await this.loadInitialTerminalData();
} finally {
    this.flushHeldData();
    this.loaded = true;
}
```

`loadInitialTerminalData()` (`termwrap.ts:729-760`) does exactly the right thing, in the right order, via two `fetchWaveFile` HTTP GETs:

1. `fetchWaveFile(blockId, "cache:term:full")` — a periodic, bounded, *serialized xterm state* snapshot (produced by `SerializeAddon.serialize()`, capturing the terminal's current scrollback/formatting/cursor as a replayable string), paired with `meta.ptyoffset` and `meta.termsize`.
2. `fetchWaveFile(blockId, "term", ptyOffset)` — the raw incremental PTY bytes *starting from that offset* — i.e. only the small delta since the last snapshot, not the whole session.

This two-tier design (bounded snapshot + small delta) is exactly right, and it's why `AgentShellSubblock` doesn't need any restore logic of its own — it gets this for free from `TermWrap.init()`, identically to the main Terminal view. **Neither call site is missing anything.**

### 1.2 Both of that mechanism's data sources are dead ends server-side

**(a) The raw incremental "term" content is never durably written — only broadcast live, once, to whoever happens to be subscribed at that instant.**

`agentmux-srv/src/backend/blockcontroller/shell/lifecycle.rs:660-666` (the PTY stdout-reader loop):

```rust
handle_append_block_file(
    broker,
    &block_id_read,
    "term",
    chunk,
    None, // PTY output is raw terminal data; no FileStore write-through
    None, // not an agent output stream; no global mirror
);
```

Inside `handle_append_block_file` (`shell/file_ops.rs:77-158`), the entire `if let Some(fs) = filestore { fs.make_file(...); fs.append_data(...) }` write-through block is skipped whenever `filestore` is `None` — only `broker.publish(event)` (the live WPS broadcast) runs. On the frontend, that broadcast lands in a plain `rxjs Subject` (`frontend/app/store/wps.ts:107-124`) — not a `ReplaySubject`/`BehaviorSubject` — so `subject.next()` delivers only to whoever is subscribed *right now* and the value is gone for any subscriber that arrives a moment later. A fresh `TermWrap`/xterm instance (after a remount) gets nothing retroactively from this channel, by design of the primitive being used.

Confirmed via `git blame`: this `None, None` predates the current module split (#1906) — long-standing, not a regression, and (per the comment) reads as a deliberate choice, most plausibly to avoid unbounded blockfile growth from long, noisy PTY sessions (e.g. `htop`, streaming logs) if raw bytes were persisted forever with no bound.

**(b) The periodic-snapshot RPC that's supposed to make (a) a non-issue is an unimplemented stub.**

The frontend already calls it — `TermWrap.processAndCacheData()` (`termwrap.ts:762-773`), invoked every ~5s via an idle-callback loop (`runProcessIdleTimeout`, `termwrap.ts:775-787`), gated so it only fires once `dataBytesProcessed` crosses `MinDataProcessedForCache` (i.e. only when there's been meaningful new output, not every idle tick):

```ts
processAndCacheData() {
    if (this.dataBytesProcessed < MinDataProcessedForCache) return;
    const serializedOutput = this.serializeAddon.serialize();
    const termSize: TermSize = { rows: this.terminal.rows, cols: this.terminal.cols };
    fireAndForget(() =>
        services.BlockService.SaveTerminalState(this.blockId, serializedOutput, "full", this.ptyOffset, termSize)
    );
    this.dataBytesProcessed = 0;
}
```

Server-side, `agentmux-srv/src/server/service/misc.rs:42-44`:

```rust
("block", "SendCommand") | ("block", "SaveTerminalState") => {
    WebReturnType::success_empty()
}
```

`state`/`ptyOffset`/`termSize` are silently discarded. The RPC's own registered doc comment (`agentmux-srv/src/backend/service.rs:151-160`) already describes the intended behavior — `"save the terminal state to a blockfile"` — it was simply never implemented, just stubbed to return success so the frontend's `fireAndForget` call doesn't error.

### 1.3 Net effect

Every ~5s of active use, the frontend tries to snapshot; the snapshot is silently dropped. Every byte of PTY output is broadcast once and never persisted. On any remount — drawer close/reopen for the agent shell, or a tab teardown/rebuild for a standalone Terminal pane — `loadInitialTerminalData()` gets a 404 on both fetches and the new terminal starts genuinely blank. Nothing is deferred or delayed; it's gone.

---

## 2. Design: implement both pieces of the already-designed two-tier mechanism

No new architecture is needed — the frontend already assumes and calls the right shape. This is a backend wiring gap, not a design gap.

### 2.1 Part A — raw `"term"` write-through (needed regardless, since delta reads depend on it)

`ShellController` (`agentmux-srv/src/backend/blockcontroller/shell/controller.rs:80-96`) already holds `wstore: Option<Arc<Store>>` (used today only to seed `cmd:cwd` on spawn) but has **no `filestore: Option<Arc<FileStore>>` field** — unlike `PersistentController`/`SubprocessController`, which already receive one via their constructors (`persistent.rs:248`, confirmed by grep). The factory that constructs controllers (`blockcontroller/mod.rs:423-437`) **already has a `filestore` value in scope** at that exact call site — it's passed to the `BLOCK_CONTROLLER_SUBPROCESS` arm a few lines below (`mod.rs:444-450`) but not to the `BLOCK_CONTROLLER_SHELL | BLOCK_CONTROLLER_CMD` arm just above it (`mod.rs:430-437`). This is the same *shape* of gap as the Windows `CREATE_NO_WINDOW` wiring gap found earlier this session (`docs/retro/retro-windows-terminal-window-leak-2026-06-21.md`) — a constructor that was never updated to receive infrastructure added after it was first written.

**Proposed change:**
1. Add `filestore: Option<Arc<FileStore>>` to `ShellController`'s constructor + struct (mirroring `PersistentController`'s existing field).
2. Thread it into the PTY-read-loop's captured `_read`-suffixed clones (alongside `block_id_read`, `broker_read`, `inner_read` — same pattern already used there).
3. In `lifecycle.rs:660-666`, replace the first `None` with `filestore_read.as_ref()`.
4. In `mod.rs:430-437`, pass `filestore` through to `ShellController::new(...)`, matching what `BLOCK_CONTROLLER_SUBPROCESS` already does just below it.

This alone makes `fetchWaveFile(blockId, "term", ptyOffset)` succeed, restoring scrollback **from `ptyOffset` forward**. With Part B not yet done, `ptyOffset` always resolves to 0 (no snapshot ever exists), so this alone means "replay the entire session's raw PTY bytes from the start" on every reconnect — functionally correct (xterm.js is well-suited to replaying a raw ANSI byte stream from scratch — this is exactly the `script`/`ttyrec`-replay model), but means reconnecting to a very long-running, high-output session (hours of `htop`, a build log tailing continuously) could be slow and the underlying `"term"` blockfile grows unbounded on disk. Both concerns are exactly what Part B exists to bound.

### 2.2 Part B — implement `SaveTerminalState` for real

**Proposed change**, in `agentmux-srv/src/server/service/misc.rs`, split the current combined `("block", "SendCommand") | ("block", "SaveTerminalState")` arm and implement the latter:

- Extract `blockId: String`, `state: String` (the serialized xterm output), `ptyOffset: i64`, `termSize` (rows/cols) from `call.args` (same `service::get_arg` pattern used by the `GetControllerStatus` arm just above).
- Persist `state` as the blockfile named `cache:term:full` (matching `TermCacheFileName` on the frontend) via `state.wstore`'s filestore — `write_file` (full-replace, not append, since this is a periodic snapshot that supersedes the previous one) for the content, `write_meta` for `ptyoffset`/`termsize` (matching exactly what `loadInitialTerminalData()` reads back: `cacheFile.meta["ptyoffset"]`, `cacheFile.meta["termsize"]`).
- Both `write_file`/`write_meta` already exist on `FileStore` (`filestore/core.rs:320`, `:543`) — no new storage-layer primitives needed.

Once both parts land, reconnect behavior becomes: restore the last snapshot (bounded — xterm's own `scrollback` option, e.g. `2000` lines per `AgentShellSubblock`'s `TermWrap` config, already caps how much a serialize can contain) instantly, then replay only the small delta since that snapshot (typically ≤5s of output, per the idle-callback cadence) — fast and correct regardless of total session length or output volume.

### 2.3 What Part B does *not* solve on its own: unbounded raw-file growth

Even with Part B implemented, the raw `"term"` blockfile from Part A keeps growing forever (nothing truncates it) — it's just no longer *read* from the start once a snapshot exists (only from `ptyOffset` onward), so the growth becomes a disk-space concern rather than a restore-correctness or restore-latency concern. Bounding/truncating the raw file (e.g. periodically deleting bytes already covered by the latest snapshot) is a legitimate follow-up but is **not required** for this fix to be correct — flagging it here so it isn't silently forgotten, not because it blocks Part A/B.

---

## 3. Scope and blast radius

- Touches: `agentmux-srv/src/backend/blockcontroller/shell/controller.rs`, `shell/lifecycle.rs`, `blockcontroller/mod.rs` (Part A); `agentmux-srv/src/server/service/misc.rs` (Part B).
- Affects **every** `view:"term"` surface in the app identically (standalone Terminal panes, the agent-shell drawer, and any future consumer of the same `ShellController`) — this is the correct blast radius; a fix scoped to only the agent-shell drawer isn't possible (and shouldn't be attempted) since the bug lives one layer below both surfaces.
- No frontend changes needed at all — `TermWrap`'s restore logic is already correct and already calls both endpoints; it just needs the server to stop 404ing.
- No new storage primitives needed — `FileStore::write_file`/`write_meta`/`append_data` already exist and are already used by this exact write-through pattern for other controllers (`PersistentController`, `SubprocessController`, `AcpController`).

---

## 4. Non-goals (this pass)

- Truncating/rotating the raw `"term"` blockfile to reclaim disk space once superseded by a snapshot (§2.3) — real, but not required for correctness; separate follow-up.
- Changing the 5s idle-callback cadence or `MinDataProcessedForCache` threshold for snapshotting — existing frontend tuning, out of scope here.
- Any change to `AgentShellSubblock`, `TermWrap`, or `term.tsx` — confirmed unnecessary; the restore logic there is already correct and shared identically by both surfaces.

## 5. Suggested implementation order

1. Part A (raw write-through) — smaller, mechanical, mirrors an existing pattern (`SubprocessController`'s filestore wiring) closely enough to copy the shape directly. Fixes the reported bug on its own, with the "replay from 0" caveat noted in §2.1.
2. Part B (`SaveTerminalState`) — bounds the fix properly (fast, small-delta reconnects regardless of session length/volume). Should follow Part A directly rather than shipping Part A alone as a final state, since Part A alone leaves every reconnect replaying the whole session.
3. (Optional, separate ticket) Raw-file truncation/rotation once a snapshot supersedes it (§2.3).
