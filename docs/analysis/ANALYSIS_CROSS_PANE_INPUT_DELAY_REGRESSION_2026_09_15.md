# Analysis: Cross-Pane Input Delay Has Returned — Root Cause on the Ingress Path

**Date:** 2026-09-15
**Status:** Implemented — §4 item 2 (the in-memory lease registry), plus the
exit-path release §5 adds. Root cause is code-inspection-confirmed and the
fix is covered by unit + real-PTY tests (each falsified: source deliberately
broken, right test observed failing, restored); it has NOT been measured
against a running build, since no dev instance was available this session.
The before/after bench in §6 is still worth running.
**Symptom (user-reported):** typing in one pane feels slow again while a
different pane is producing output — the same user-facing symptom
`ANALYSIS_CROSS_PANE_INPUT_DELAY_UNDER_OUTPUT_LOAD_2026_09_04.md` (the "09-04
report") diagnosed and (partially) fixed.

---

## 1. Summary

**The 09-04 fix is intact and unregressed.** `fair_drain_priority` /
`priority_pane_key` / the `pending_priority` trickle-flush loop in
`agentmux-srv/src/server/websocket.rs` are all present, unchanged in
substance, and still doing what the analysis and its PR (#2973, plus the
Codex/ReAgent hardening round documented at the top of that file) describe.
I diffed every commit that has touched `websocket.rs` since #2973 landed
(8 commits, up to today's `10d7ab35d`) — none of them touch the fair-drain
machinery.

**A different, newly-introduced bug reproduces the same symptom through a
different mechanism.** `PR #3194` ("PtyShell attaches to the pane's real
visible shell, with a lease-based lock", merged 2026-09-11) added a
synchronous, un-`spawn_blocking`'d SQLite read to the `controllerinput`
handler — the RPC handler that processes **every keystroke typed into every
pane on every connection**. That handler now does:

```rust
// agentmux-srv/src/server/websocket.rs:1049-1059
let locked = wstore
    .get::<Block>(&cmd.blockid)
    .ok()
    .flatten()
    .and_then(|b| b.meta.get(crate::server::META_KEY_AGENT_LOCK_UNTIL).and_then(|v| v.as_i64()))
    .map(|until| until > agentmux_common::time::now_ms())
    .unwrap_or(false);
```

`wstore` is `agentmux-srv/src/backend/storage/store.rs`'s `Store` —
**a single process-wide `SQLite` connection behind one `Mutex<Connection>`**
("Uses `Mutex<Connection>` matching Go's `MaxOpenConns(1)`", the module's own
doc comment). `Store::get` is a plain synchronous function: it blocks on
that mutex, then blocks on a real SQLite query, with no `.await` anywhere in
the call — and it is invoked directly inline inside the `controllerinput`
async task body, not wrapped in `tokio::task::block_in_place` or
`spawn_blocking`.

This is exactly the risk class the 09-04 report named and warned about in
its own §3.4 ("General risk class: a blocking call inside an async task
stalls the whole connection's egress") — the same class of bug that once hit
`sysinfo.rs` and was fixed in commit `0f34704a8` (#1782). PR #3194
reintroduced a new instance of it, on a hot path the sysinfo fix never
touched, four days ago — a week after the 09-04 report was written, so it
isn't something that report could have caught.

---

## 2. Why this is worse than the sysinfo instance, and how it produces cross-pane symptoms specifically

The sysinfo bug stalled *one connection's* egress loop for the duration of a
`/proc` scan. This one is structurally broader in two ways:

1. **It's on the keystroke path, not a periodic timer.** Every single
   `controllerinput` RPC — i.e. every keystroke in every terminal and every
   PtyShell-backed pane — now pays a synchronous SQLite round trip before
   the input is even parsed and forwarded to the PTY. That's added latency
   to typing in general, independent of any other pane.
2. **The lock it contends on is process-wide, not per-connection.**
   `Store`'s single `Mutex<Connection>` is shared by *every* other
   synchronous `Store` operation happening anywhere in the process at that
   moment — other panes' `controllerinput` calls, `PtyShellInput`/
   `PtyShellResize`'s own `lock_shell_for_agent` write (`server/mod.rs:1814`,
   also unwrapped, also on `wstore`/the same `Store`), background-task
   bookkeeping, ambient-narration dedupe checks, dock snapshots, and turn
   bookkeeping in `persistent.rs`. A keystroke in pane B now queues behind
   *any* of that, not just behind pane A's own PTY output — which is a
   strictly larger set of triggers than the 09-04 report's dominant cause
   (a single connection's WS egress FIFO), and reproduces as the same
   "typing near a busy pane feels laggy" complaint.

Concretely: an agent actively driving its own visible shell via the new
`PtyShell*` MCP tools (PR #3194's whole feature) calls
`lock_shell_for_agent` — an unwrapped synchronous `Store` write — before
**every** `PtyShellInput`/`PtyShellResize` call. If that agent is issuing
those in any kind of burst (which the feature exists to allow — an agent
driving a real interactive shell), a human typing in a *different* pane
will have their own `controllerinput` handler's `Store` read serialize
behind that agent's `Store` writes, on the exact same global mutex. Two
pieces of the same PR contend with each other.

---

## 3. What's NOT the cause (checked and ruled out)

- **Egress fan-out** (09-04's root cause): unregressed, see §1.
- **Frontend main-thread blocking** (09-04 §3.5, the streaming-markdown
  fix): `frontend/app/view/agent/components/MarkdownBlock.tsx`'s
  `STREAM_RENDER_MS = 90` throttle is still in place, unmodified.
- **Background-lane starvation / coalescing** (`coalesce_background`,
  `websocket.rs:368-385`): unchanged.
- **PTY output persistence itself** (`handle_append_block_file`,
  `agentmux-srv/src/backend/blockcontroller/shell/file_ops.rs`): writes
  through `FileStore`, a *separate* SQLite file with its own
  `Mutex<Connection>`, not `Store`'s. High-volume PTY/agent output does not
  contend `Store`'s mutex through this path. `resolve_global_output_zone`
  (the one place this file touches `wstore`/`Store`) is resolved **once**
  per reader-task setup (see its call sites' own "resolved once" comments
  in `host_spawn.rs`/`persistent.rs`), not per chunk — so ordinary output
  streaming, by itself, does not flood `Store`'s mutex either. The
  contention is specifically between `controllerinput`/`PtyShellInput`'s new
  `Store` traffic and any of `Store`'s other legitimate (pre-existing)
  callers, not between "output volume" and "input" in general the way the
  09-04 symptom's literal description suggests.

---

## 4. Recommended fix

1. **Immediate, matches existing precedent:** wrap the `wstore.get::<Block>`
   call in `controllerinput` (`websocket.rs:1049`) and the `Store` write in
   `lock_shell_for_agent` (`server/mod.rs:1814`) in
   `tokio::task::block_in_place` — the exact fix already applied to the
   sysinfo instance of this same bug class (#1782). Minimal, safe, matches
   established pattern.
2. **Better, avoids paying a DB hit on every keystroke in the common case:**
   most panes are never touched by a `PtyShell*` call and so never have
   `term:agentlockuntil` set at all. Cache "this block currently has an
   active agent lock" in memory (e.g. alongside `ShellControllerInner`'s
   existing per-block state, which is already behind its own per-pane lock
   per the 09-04 report's §2) instead of round-tripping to `Store` on every
   keystroke regardless. This removes the `Store` contention entirely for
   the overwhelmingly common case instead of just making it non-blocking.
3. **Bench coverage gap:** `tools/tests/bench-term-cross-pane.mjs` (built to
   validate the 09-04 fix) measures pane B's echo latency while pane A
   floods raw PTY *output* (`yes`) — it does not exercise concurrent
   `controllerinput`/`PtyShellInput` traffic, so it would not have caught
   this. Extend it (or add a sibling script) with a scenario where pane A is
   driven via rapid `PtyShellInput` calls (or just rapid synthetic keystrokes
   into A) while B's echo latency is measured, to give this class of bug the
   same regression coverage §5-item-5 of the 09-04 report gave the egress
   side.

## 5. What was implemented

Took §4 item 2 (the in-memory route) rather than item 1's `block_in_place`
wrap, because item 1 only makes the blocking call *politely* blocking — it
still pays a SQLite round trip and still serializes on `Store`'s global
mutex for every keystroke in every pane, which is the wrong cost to pay for
a question that is almost always "no agent is driving any shell right now."

- **New `backend/blockcontroller/agent_lock.rs`** — the lease's authoritative
  copy: `block_id → expiry_ms` in a `HashMap` behind a `Mutex` held for
  nanoseconds with no I/O inside. `is_locked` prunes lapsed entries as it
  reads them, so the map is bounded by the number of *currently* leased
  blocks (normally zero), not by every block ever leased.
- **`controllerinput` (`server/websocket.rs`)** reads that instead of
  `wstore.get::<Block>()`. No SQLite, no `Store` mutex, no disk on the
  keystroke path.
- **`lock_shell_for_agent` / `handle_pty_shell_stop` (`server/mod.rs`)**
  write and clear the registry *first*, then do the `term:agentlockuntil`
  meta write as before. Meta is now explicitly the frontend-badge copy
  (`AgentShellSubblock.tsx`), which is what that handler's own comment
  already claimed it was ("the frontend gate is the UX layer... not the
  correctness boundary") — it just wasn't true of the enforcement path yet.
  Locking in memory first also shrinks the pre-existing "human keystroke in
  the sliver before the lock write commits" race from a DB commit's latency
  to a mutex acquire's.

### The `exit` half

Enforcement living in memory is also what makes the lease safe around a
human typing `exit`, which the close-on-exit work
(`SPEC_TERM_EXIT_RESPAWN_LOOP_2026_09_15.md` §10) just made the default for
shell panes. Three properties, each with a test:

1. **A shell that exits releases its lease immediately** —
   `shell/lifecycle.rs`'s wait/cleanup task calls `agent_lock::release`
   alongside publishing `STATUS_DONE`. Without this, a pane whose agent
   wrote to it within the last 4s keeps dropping the human's keystrokes
   after the PTY is already gone. Covered by
   `an_exited_shell_does_not_keep_the_agent_lease` (real-PTY, `#[ignore]`d
   like its siblings).
2. **A lease always lapses on its own**, with nobody calling `PtyShellStop`
   — an agent that crashes mid-write cannot leave a human unable to type
   `exit` in their own shell. Covered by
   `controllerinput_flows_again_once_the_agent_lease_lapses`.
3. **A stale `term:agentlockuntil` in meta locks nothing by itself.** This
   is the deliberate fail-open direction: meta can outlive the process that
   wrote it (srv restart), memory cannot. Asserted inside
   `controllerinput_is_dropped_while_an_agent_lock_is_active`.

Not changed: whether a *live* lease should drop human input at all. It
should — that's the write-collision guard the feature exists for, it's
bounded at 4s, and the only panes that can be leased (an agent's own
composer-drawer shell, per `is_owned_by_agent`) already render the "Agent is
using this shell" badge while it's held. A standalone Terminal pane can
never be leased, so nothing about typing `exit` there was ever gated on any
of this.

## 6. Validation still needed

The root cause and the fix are both code-level — inspected, implemented, and
covered by falsified tests — but **neither has been measured on a running
build**: no dev instance was available this session (`authkey.dev` not found
under `%APPDATA%`), so there is no before/after P95 number the way the 09-04
report has one.

What's left, in order:

1. Build the bench extension in §4 item 3 (concurrent input traffic, not just
   an output flood — the existing `bench-term-cross-pane.mjs` cannot see this
   class of bug by construction).
2. Run it against `main` (pre-fix) and against this branch. The claim to
   check is that pane B's keystroke P95 stops tracking other panes'/agents'
   `Store` activity — not merely that it improved.
3. Manually re-confirm the `exit` path end to end in a real pane: an agent
   drives the composer-drawer shell, the human types `exit` immediately
   after, the pane closes rather than swallowing the keystrokes. The
   `#[ignore]`d real-PTY test covers the backend half of this; it does not
   exercise the frontend badge/gate or the close-on-exit saga.
