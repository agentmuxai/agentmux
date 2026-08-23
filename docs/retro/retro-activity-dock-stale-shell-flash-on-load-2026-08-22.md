# Retro: Activity Dock flashes stale shell rows on every pane load

**Date:** 2026-08-22
**Severity:** Low for the original bug — cosmetic only, no data loss, nothing
re-executed. It's the second time this exact symptom has been reported by the
user (first as "old long-running processes that are socked" while diagnosing
a separate cross-channel resume bug, then again directly during manual
verification of that bug's fix). Note the FIX's first draft briefly
introduced something more severe — see "Review findings" below — caught and
closed before merge.
**Observed by:** Camper (Claude agent), reported directly by the user during
manual testing of PR #2755: "what about the dock items? they show and then
disappear."
**Related specs:** `SPEC_LONG_RUNNING_SHELL_PINNED_DOCK_2026_06_15.md`,
`SPEC_PERSISTENT_SHELL_NODE_2026_06_11.md`
**Related retros:** `retro-agent-resumed-9-day-stale-session-2026-08-22.md`
(the cross-channel resume bug this was discovered while verifying — a
different bug, same session, same user-reported "old garbage on load"
language)

---

## TL;DR

Reopening an agent pane briefly showed old, already-finished shell commands
in the Activity Dock as if they were live and running, then made them vanish
a moment later. Confirmed by direct user testing (not a hypothesis): the
flash happened right on pane load, and the items shown were "old/unfamiliar"
— not current activity. Root cause: `shell_node_create`, the WPS event the
frontend uses to learn about a persistent shell, is published exactly once
per shell and replayed verbatim (persist:64 ring) on every pane
mount/reconnect, for every shell in the block's recent history — including
ones that finished hours or days ago. The event payload carries no status
field, so the frontend always built a `ShellNode` with `status: "running"`
regardless of whether it was a genuinely live spawn or a replay of a
long-dead shell. The correction — an "exit" event replayed from that same
shell's own separate `shell:<id>` ring — arrives moments later via an
independent subscribe+replay round trip, at which point the Activity Dock's
existing departure-flash animation plays and the row disappears. Fixed by
adding a `shellstatus` RPC command that queries the shell's TRUE current
state (already tracked server-side in `ShellSessionRegistry`) and firing it
immediately after every `shell_node_create`, correcting the dock the instant
the (much cheaper) status lookup resolves — in practice almost always before
the "running" row the create event queued has painted at all. Node creation
itself is unchanged (still always "running" at that point) specifically to
avoid introducing any new risk of a chunk/exit event racing ahead of its
node's own existence in the document.

---

## What Happened

### The report

Mid-session, right after manually verifying the (unrelated) cross-channel
resume fix in PR #2755 by reopening an agent pane, the user asked directly:

> "ok, I tried it with Agent2 and it was faster .. but what about the dock
> items? they show and then disappear"

This echoed almost verbatim the very first complaint that had kicked off
this whole session's cross-channel-resume investigation: "it is showing
garbage, like old long-running processes that are socked." That earlier
retro flagged the dock symptom as a **lower-confidence, unconfirmed**
secondary observation and explicitly deferred investigating it. This session
picked it back up on the user's direct follow-up.

Before proposing any fix, two clarifying questions confirmed this wasn't the
dock's own intended behavior (terminal rows are *supposed* to fade out after
a retention window — that's a feature, not a bug):

- **Timing:** "Right on open/load" — not after sitting visible for a while.
- **Content:** "Old/unfamiliar" — not current, recent activity.

Both answers ruled out the benign explanation and confirmed the same failure
class the original bug report described.

### Finding the mechanism

`frontend/app/view/agent/components/ActivityDock.tsx` renders shell/subagent/
tool rows derived from the document's `nodes()`. Shell nodes specifically are
**not** part of the persisted document-history snapshot at all (confirmed:
zero references to "shell" in `parseHistoryLines.ts`) — they're reconstructed
entirely from a live WPS event stream, `useShellNodeStream.ts`.

That stream subscribes to a block-scoped `shell_node_create` event with
`persist: 64` (`agentmux-srv/src/server/mod.rs`'s `handle_shell_create`) —
meaning the last 64 creation events for this block replay to any new
subscriber, by design, "so multiple shells in a pane all replay on WS
reconnect / pane remount" (the mechanism's own doc comment). Each replay is
handled identically to a live spawn:

```ts
const node: ShellNode = {
    ...
    status: "running",   // ALWAYS — the event payload has no status field
    spawnedAt: d.timestamp ?? Date.now(),
    ...
};
opts.queue.pushShellCreate(node);
subscribeShellScope(shellId);   // establishes this shell's OWN per-shell ring
```

For an already-long-exited shell, the correction only ever arrives via that
shell's own separate `shell:<id>` ring (`persist: 1024`), which carries its
`exit` chunk. Subscribing to it is a **second, independent** subscribe+
replay round trip — not simultaneous with the create event's own replay.
Concretely, on every pane load with any shell history: create-event replay
lands first → dock renders the row as "running" → moments later the
per-shell exit-event replay lands → `ShellStatusUpdate` flips it to
terminal → the dock's existing `EXIT_FLASH_MS` departure animation plays →
the row disappears. Exactly "shows on load, old/unfamiliar, then
disappears."

This was independently confirmed by tracing the exact code paths (not
inferred from behavior alone): `handle_shell_create` publishes
`shell_node_create` with no status field, ever, for the shell's entire
lifetime; `ShellSessionRegistry` (the actual source of truth for a shell's
live/exited state) was never consulted by the frontend at creation time at
all.

---

## The Fix

Added a `shellstatus` RPC command
(`agentmux-srv/src/backend/rpc_types/{block,commands}.rs`,
`server/shell_handlers.rs`) that thinly wraps the already-existing, already-
tested `ShellSessionRegistry::get_status` — the same registry the HTTP-only
`POST /api/v1/shell/status` route (used by the MCP server) already reads,
just exposed on the RPC channel the frontend actually uses.

`useShellNodeStream.ts`'s `shell_node_create` handler now fires
`ShellStatusCommand` immediately after queuing the node (still always
created as `"running"` first — unchanged from before) and after establishing
the per-shell subscription. If the check reports the shell already exited,
it immediately pushes a correction via the **same, already-tested**
`pushShellExit` → `ShellStatusUpdate` path the real exit-chunk replay would
eventually use anyway — just arriving via a much cheaper single-registry-
lookup round trip instead of a full subscribe+ring-replay one, so in
practice it wins the race and the "running" row this pushShellCreate just
queued never actually paints.

**Deliberately did NOT change node-creation ordering.** An earlier draft of
this fix tried to defer `pushShellCreate` itself behind the status check —
i.e., don't insert the node at all until we know its real status. Rejected
before landing: `subscribeShellScope` (which starts loading this shell's own
chunk/exit ring in parallel) is called synchronously either way, and
`ShellChunkAppend`/`ShellStatusUpdate` are no-ops in the reducer when they
can't find a matching node id. Deferring creation would have raced the two
independent async round trips (the status check vs. the chunk-ring replay)
against each other with no ordering guarantee, risking silently dropping a
shell's captured output if the ring replay won that race and arrived before
the node existed — trading a cosmetic flash for a real data-loss risk. The
shipped fix avoids this entirely: creation is always synchronous and
unchanged, and the status check only ever adds a *faster corrective*
follow-up on the existing, safe path.

The proxy `exitedAt` used for the correction (the shell's own creation
timestamp — `ShellStatusResponse` doesn't carry a real exit time) is
intentionally approximate: for the case this fixes (a shell that finished
long ago), it's already far outside the dock's retention window either way,
so the row renders as invisible the instant the correction lands. The real
`exitedAt`/exit code are refined moments later regardless, once the shell's
own exit-chunk replay arrives on its own — same as before this fix, just no
longer the ONLY path to a correct status.

**Tests:**
- Backend (`server/shell_handlers.rs`, 4 new): the `shellstatus` RPC handler
  reports `running: true` for a live shell, `exited-ok`-shaped data (exit
  code 0) and `exited-err`-shaped data (nonzero code) for exited shells, and
  `running: false` / no exit code for an unknown id — exercised through the
  real `WshRpcEngine` dispatch path, not just the underlying registry call.
- Frontend (`useShellNodeStream.test.ts`, 5 new): the pure
  `shellStatusCorrection` decision function — still-running and failed-check
  both correctly no-op; clean exit, nonzero exit, and missing-exit-code all
  map to the right terminal status/exit code.

Full verification: `cargo test -p agentmux-srv` (2750 passed, 0 failed),
`npx tsc --noEmit` (clean), `npx vitest run` (2965 passed; one unrelated
`tool-renderers/registry.test.ts` timeout confirmed as a pre-existing flake
under full-suite parallel load — passes cleanly in isolation, no relation to
this change).

---

## Review findings (PR #2770)

ReAgent's review of the initial fix caught a real regression before merge —
worse than the bug this PR set out to fix, and correctly blocked it:

**P1 — a genuinely live shell could be misreported as failed for its entire
run.** `handle_shell_create` (`server/mod.rs`) publishes `shell_node_create`
to the frontend, then `tokio::spawn`s the runner task fire-and-forget —
`ShellSessionRegistry::register_full` (the call that actually creates the
registry entry `get_status` reads) only happens once that task reaches it,
AFTER spawning the real child process. There is no ordering guarantee
between "frontend receives shell_node_create and fires its status check"
and "runner reaches register_full." If the status RPC round-trip completed
first, `get_status` returned its "unknown" default (`running: false,
exit_code: None`) — byte-for-byte identical to the shape a genuinely
already-exited shell produces. `shellStatusCorrection` then pushed a false
`exited-err` correction for a real, live, freshly-spawned shell (e.g. an
actual `task dev`). Since nothing in the reducer ever restores a status
once set (`ShellChunkAppend` only appends log content, never flips status
back), that shell would show as failed in the Activity Dock for its ENTIRE
real run, until/unless it happened to exit for real later.

**Fix:** added `ShellSessionRegistry::get_status_if_known`, returning
`Option<ShellStatusInfo>` — `None` when no registry entry exists at all,
distinct from `Some(status)` with `running: false`. The existing
`get_status` (used by the MCP-facing `POST /api/v1/shell/status` HTTP
route) is untouched — that route's documented contract already treats an
unrecognized id as "not running," which is the correct answer for an agent
calling `ShellStatus` on an id it made up, and changing it would be a
breaking change to an unrelated caller. The NEW `shellstatus` RPC command
(the only consumer that needs the three-way distinction) reports a `known`
boolean; `known: false` means "don't correct" — the frontend's
`shellStatusCorrection` now takes `{ known, running, exit_code? }` and
returns `null` (no correction) whenever `!known`, exactly like it already
did for a fully-failed RPC call. This closes the race safely: a live shell
caught mid-registration now correctly falls back to "stay running, let the
real exit-chunk replay correct it later if needed" — the same degraded-but-
safe behavior this PR started from for the narrow cases it doesn't fully
eliminate (see Prevention/Follow-ups below), rather than a false positive.

Added 1 backend test (`shellstatus_unknown_id_reports_known_false`,
replacing the now-redundant original unknown-id test) confirming `known:
false` for any id without a registry entry, and updated all existing
backend/frontend tests for the new `known` field. Re-verified: `cargo test
-p agentmux-srv` (2750 passed), `npx tsc --noEmit` (clean), `npx vitest run`
(new shellStatusCorrection tests: 5/5 passed).

**Round 3 — P1, same underlying pattern, a different pair of racers.**
ReAgent's re-review of the round-2 commit (and, per its own report,
matching a finding chatgpt-codex-connector had already left on the same
line) caught that the fix STILL had an unguarded race: the synthesized
`ShellStatusCommand` correction and the shell's REAL exit/stop event
(delivered independently via the already-subscribed `shell:<id>` chunk
ring) have no ordering guarantee relative to EACH OTHER either. If the real
event won that race — the shell genuinely exits or is stopped by the user
before the status RPC resolves — the correction callback still fired
unconditionally once its promise settled, overwriting the already-correct
row (right status, including `"stopped"`, which `ShellStatusResponse` has
no way to express at all — only `exited-ok`/`exited-err`; and the real
exact `exitedAt`) with a stale, less-accurate synthesized one (wrong
status, `exitedAt` hardcoded to `spawnedAt`). `ShellStatusUpdate` in the
reducer overwrites a node's status unconditionally, so nothing would have
undone the corruption on its own.

**Fix:** a `reallyResolved` `Set<string>` in the hook's closure, populated
the instant a shell's real exit/stop event lands (`handleShellChunk`'s
`exit` branch). The correction callback checks it FIRST, before applying
anything: if the real event already landed, skip — trust it, never
overwrite. Safe by construction under JS's single-threaded execution model:
the check and the (conditional) push both run synchronously within the
same callback invocation, so no other event handler can interleave between
"check the set" and "act on it." The reverse order (correction lands first,
real event arrives later) was already safe without this guard — the real
event's own unconditional overwrite is exactly the desired behavior when
authoritative data arrives after a synthesized guess.

Added a genuine ordering test (`useShellNodeStream.test.ts`, new
`describe` block, mocking `waveEventSubscribe`/`ShellStatusCommand` and
controlling the status-RPC promise's resolution timing directly) proving
the guard actually suppresses the stale correction — not merely that the
two happen to agree: the test resolves the status check with a
DELIBERATELY wrong "already exited, code 1" reading after the real
`stopped` event has already landed, and asserts `pushShellExit` was still
called exactly once (the real one). A second test confirms the correction
still applies normally when it resolves with no real event racing it at
all. Re-verified: `cargo test -p agentmux-srv` (2750 passed), `npx tsc
--noEmit` (clean), `npx vitest run` (7/7 in this file, including the 2 new
ordering tests).

---

## What This Is NOT

- Not a re-execution or duplication of any shell command — purely a display
  artifact. The shell's actual process lifecycle (`ShellNodeRunner`,
  `ShellSessionRegistry`) was never affected by this bug.
- Not the same bug as `retro-agent-resumed-9-day-stale-session-2026-08-22.md`
  — that was a backend session-id resume correctness bug (a wrong
  conversation resuming), fixed in PR #2755. This is a frontend display-
  timing bug (a right conversation, with a cosmetically-wrong dock flash) —
  the two were reported by the user in very similar language ("old...
  garbage"/"old long-running processes") because both manifest as stale-
  looking state on pane load, but they are unrelated code paths with
  unrelated fixes.
- Not a data-loss risk introduced by the fix itself — see "Deliberately did
  NOT change node-creation ordering" above for why the safer, slightly less
  aggressive design was chosen over a naive "just don't render until we
  know the real status" approach.

## Prevention / Follow-ups

- Not tracked as a further issue: the fix reduces the flash window to
  "however long a single registry lookup takes" rather than eliminating it
  by a hard guarantee — a sufficiently slow/contended status RPC could in
  principle still lose the race against a fast chunk-ring replay and let one
  frame paint. Given the cosmetic-only severity and that a guaranteed fix
  would require either a new "pending" ShellNode status variant (rippling
  through the dock's status union and retention logic) or reworking the WPS
  persist-ring replay itself to carry live status, this was judged
  disproportionate for what is now a much-narrower remaining window rather
  than a near-certain flash on every load.
