# SPEC: Continuous session persistence + trustworthy shutdown

**Date:** 2026-09-08
**Status:** Proposed
**Author:** Camper@narko
**Tracks:** the gap left by `SPEC_SESSION_RESTORE_AND_SAVED_LAYOUTS_2026_08_13.md`
Feature 1 (shipped) — see §2 for exactly what is already built and what is new here.

## 0. TL;DR

Session restore works — it shipped 2026-08-14 (PR #2560) and correctly
restores a workspace when the user closes AgentMux and reopens it. But it
writes **once**, on that graceful close, and **the OS is never able to ask for
that write**: AgentMux does not handle `WM_QUERYENDSESSION`/`WM_ENDSESSION`
at all, so a Windows restart terminates the process without the snapshot ever
being taken. Three further properties make even the wired-up path
untrustworthy: it never runs on a crash, it can be outrun by process exit, and
when it fails it fails silently.

This spec (1) makes the app respond to OS shutdown at all, (2) makes the
session record **continuous** — bounded crash-loss instead of total — and
(3) makes the shutdown path that flushes it **actually finish before the
process dies**. It deliberately reuses two auto-save patterns already proven
elsewhere in this codebase rather than inventing a third.

**Ship order matters**: Phase 0 (§4.0) is small, self-contained, and closes
the most surprising inconsistency — closing the app preserves your workspace,
but restarting the machine silently does not.

**Note on provenance (§1):** the incident that prompted this spec turned out
to be per-build channel isolation working as designed, not a defect. The gaps
described here are real and independently verified, but were found *while*
investigating that, not *because* of it.

## 1. How this spec came about (2026-09-08)

**The incident that prompted this spec turned out not to be a bug at all** —
recorded honestly here, because the gaps it led to are real and worth fixing
on their own merits, and because a reader who assumes the incident *was* the
bug will misjudge the priorities below.

What happened: a machine was rebooted mid-work; AgentMux was reopened at
"the same version, 0.55.39"; the workspace was gone. **Actual cause: a
different per-build channel was opened.** Two distinct 0.55.39 builds existed
(`local-main-b28b7a-2cd6e3d7` and
`local-lark-gated-migrations-no-au-e33013-2cf871a0`), and per-build data
isolation is documented, intended behavior — `CLAUDE.md`: *"each build is its
own AgentMux instance: its own data dir… pane layout + memories start
fresh."* Same version string, different build, different data dir. Nothing
malfunctioned.

Worth noting as a UX observation rather than a defect: "same version" is a
reasonable thing for a user to check and an insufficient one to rely on —
the version is not the instance identity. Whether that deserves surfacing in
the UI is out of scope here.

**But investigating it surfaced two genuine, independent gaps**, both
verified against the code and one verified against live data:

1. **Restore-on-relaunch works and was never in question.**
   `feat(window): restore last session's tabs/panes on relaunch (#2560)`
   merged **2026-08-14** and does exactly what users expect for
   close→reopen. (An earlier draft of this spec wrongly attributed a user's
   correct memory of this feature to the pre-2026-07-16 leaked-window bug in
   `retro-pane-layout-restore-was-a-leak-not-a-feature-2026-08-13.md`. That
   retro is accurate but describes an earlier era; the feature is real.)
2. **Gap 1 — the OS cannot ask us to save.** `WM_QUERYENDSESSION` and
   `WM_ENDSESSION` appear **nowhere** in the Rust sources. On a Windows
   restart the process is simply terminated, `backend_close_window` is never
   called, and `save_last_session_snapshot` never runs. Same class of gap on
   macOS (`applicationShouldTerminate:`,
   `NSWorkspaceWillPowerOffNotification`); Linux is partially covered only
   because the Unix path handles SIGTERM. **This would bite a user who
   reboots and reopens the *same* build** — it just wasn't what happened
   here.
3. **Gap 2 — even the wired-up path is not reliably durable** (§3): one
   point-in-time write, outrunnable by process exit, silent on failure.

### 1.1 Gap 1 confirmed against live data, not just code

The instance killed by that reboot was inspected directly. Its database
(`local-main-b28b7a-2cd6e3d7/versions/0.55.39/data/db/objects.db`) shows the
unambiguous signature of a process terminated with no graceful close:

```
session:last_topology  => ABSENT      (snapshot never written)
meta keys              => []          (meta entirely empty)
windowids              => ['d354d7fa…']  (never emptied)
db_window: 1   db_workspace: 1   db_tab: 3   db_block: 22
```

`windowids` never emptied means the `client.windowids.is_empty()` gate that
triggers the snapshot never fired, and `delete_workspace` never ran — the
whole workspace is still sitting there frozen mid-session. So Gap 1 is not
theoretical: an ungraceful kill demonstrably prevents the snapshot. It simply
was not the cause of the reported symptom.

Incidental finding worth its own consideration: those 22 orphaned blocks are
**still readable**. Post-2026-07-16 the app correctly no longer treats
leftover live rows as a restore mechanism, and no snapshot exists either — so
the data is intact but unreachable through any supported path. A "recover an
orphaned workspace" affordance may be worth having; out of scope here.

## 2. What already exists — do not rebuild these

Accuracy matters here because the shipped status of the prior spec is
mislabeled in its own header.

| Thing | Status | Where |
|---|---|---|
| Restore-on-relaunch (Feature 1 of the 08-13 spec) | **Shipped** (PR #2560) despite that spec's header still saying "Proposed — not yet implemented" | `agentmux-srv/src/server/service/session_restore.rs` (875 lines), called from `window_close.rs:143,229` and `window_create.rs:56` |
| Snapshot storage | **Shipped** | JSON blob at `Client.meta["session:last_topology"]` — `SNAPSHOT_META_KEY`, `session_restore.rs:41`. Generic `meta` sidecar, no schema migration needed |
| Named saved "Layouts" (Feature 2 of the 08-13 spec) | Not built | Proposed only; **out of scope here** (§6.2) |
| Shutdown countdown + progress modal | **Proposed, not built** | `SPEC_SHUTDOWN_COUNTDOWN_MODAL_2026_09_04.md` (PR #2981) |
| Continuous/debounced auto-persist of the *layout tree* | **Shipped** | `frontend/layout/lib/layoutPersistence.ts:396-420` — 100ms debounce |
| Interval + dirty-gated snapshot with serialized writer | **Shipped** | `frontend/app/view/agent/hooks/useSnapshotPersistence.ts:43,51,73,139` — `SNAPSHOT_INTERVAL_MS = 30_000` |

**The new work in this spec is narrow and specific**: make the *session
topology* record follow the cadence those last two rows already established
for other state, and make the close path that flushes it trustworthy.

## 3. Why the current single write is not enough

All three are real, verified in the current code — not hypothetical.

### 3.0 The OS never gets to tell us it is shutting down

Listed first because it is the highest-value, most self-contained fix in this
document. Confirmed against live data in §1.1, not merely inferred from code.

`WM_QUERYENDSESSION` and `WM_ENDSESSION` appear **nowhere** in the Rust
sources. On a Windows restart or shutdown, the OS sends these to top-level
windows precisely so an application can persist state before dying; with no
handler, the default handling lets the session end and the process is
terminated without `backend_close_window` ever being called.

The fix is small and does not depend on any other phase here: handle
`WM_QUERYENDSESSION` (return TRUE — we do not want to block shutdown, and
`ShutdownBlockReasonCreate` is the wrong tool for a save this fast), and use
`WM_ENDSESSION` as a synchronous flush point for the session snapshot. Note the
OS grants only a **bounded window** (historically ~5s, and Windows will kill
apps that dawdle), which is an argument for Phase A: a continuously-maintained
record needs only a fast final flush, not a full snapshot build, inside that
window.

Platform parity:

| Platform | Signal | Status today |
|---|---|---|
| Windows | `WM_QUERYENDSESSION` / `WM_ENDSESSION` | **Unhandled** |
| macOS | `applicationShouldTerminate:`, `NSWorkspaceWillPowerOffNotification` | Unhandled (the macOS `terminate:`/Cmd+Q path is separately known to bypass `do_close` — `agentmux-cef/src/macos_menu.rs:325`) |
| Linux | SIGTERM | Partially covered — the Unix graceful path handles SIGTERM (1500ms grace, then SIGKILL) |

### 3.1 It never runs on a crash, OOM, reboot, or force-quit

`snapshot_workspace` + `save_last_session_snapshot` are called from exactly one
place each in the close handler (`window_close.rs:143` divergence path,
`:229` normal path), both gated on `client.windowids.is_empty()` — i.e. only on
the close that empties the last window. Any exit that does not route through
that RPC writes nothing. There is **no periodic or change-triggered write path
anywhere in `session_restore.rs`** — confirmed: its only public write entry
points are `save_last_session_snapshot` (`:197`) and
`clear_last_session_snapshot` (`:229`).

Consequence: crash-loss is **total**, not bounded. Contrast
`useSnapshotPersistence.ts`, whose header comment states its explicit design
goal — "bounds crash-loss to ~30s."

### 3.2 The write can be outrun by process exit

Per `SPEC_SHUTDOWN_COUNTDOWN_MODAL_2026_09_04.md`'s own trace of the current
path: the host gives srv a **2000ms** TCP timeout to acknowledge
`backend_close_window` before posting `QuitMessageLoopTask`, while srv's
teardown grace is **5s per shell** (`KILL_GRACE_SECS`). srv can therefore still
be working when the host has already moved on to exiting.

On **Windows this is worse than a race — it is unconditional**: host exit is
`child.kill()` on srv followed by `TerminateProcess` on self. No SIGTERM, no
graceful RPC, no wait. The Unix path is genuinely graceful (SIGTERM → 1500ms →
SIGKILL). This is tracked as issue #2979 and explicitly left unfixed by the
countdown-modal spec, which candidly notes that narrating "stopping your agents
cleanly" on Windows today would be **false**.

### 3.3 When it fails, nothing says so

`save_last_session_snapshot` is documented and implemented as best-effort: a
failed `with_tx` write is `tracing::warn!`'d and swallowed, and the close
handler returns `success_empty()` regardless of whether the snapshot or the
subsequent `delete_workspace` saga succeeded. A user whose session silently
failed to save is told nothing, and finds out only on next launch.

## 4. Design

Four phases, independently shippable, in dependency order. Phase 0 alone
fixes the motivating incident; Phase A delivers most of the remaining value.

### 4.0 Phase 0 — handle OS shutdown notifications

**Ship this first.** It is the smallest change here and independently
valuable even if nothing else in this spec is ever built: today, an OS restart
loses a workspace that a manual close would have preserved, which is a
surprising and hard-to-explain inconsistency for users. (It does *not* fix the
§1 incident — nothing needs to, that was intended behavior — but it is
verified real per §1.1.)

Handle `WM_QUERYENDSESSION`/`WM_ENDSESSION` on Windows and
`applicationShouldTerminate:`/`NSWorkspaceWillPowerOffNotification` on macOS,
routing both into the same snapshot-flush the graceful close already performs
(§3.0). Ordering note: this must flush the snapshot but must **not** attempt
the full `delete_workspace` teardown cascade — the OS is about to reclaim
everything anyway, and the 5s-per-shell grace will not fit in the OS's
shutdown budget. Save, then let the process die.

### 4.1 Phase A — continuous session record

**Mechanism:** promote the session snapshot from write-once-at-close to a
debounced, dirty-gated, continuously-maintained record.

Mirror `useSnapshotPersistence.ts`'s proven shape rather than inventing one:

- **Trigger on change, not only on a timer.** The topology-affecting mutations
  are already discrete, already flow through the reducer, and already publish
  events: window open/close, tab create/close/reorder/activate, block
  create/close, and layout-tree changes. Mark the session record dirty on those
  events specifically — not on every store write (agent output, transcript
  appends, and terminal scrollback must not trigger session writes; see §6.1).
- **Debounce, don't write per-event.** A drag-resize or a rapid tab reorder
  produces bursts. `layoutPersistence.ts` already settled on **100ms** for the
  layout tree; the session record is coarser and higher-cost, so use a longer
  debounce (**proposed: 1000ms**, §7 Q1) with a **hard ceiling** — if the record
  has been dirty for longer than `SESSION_MAX_DIRTY_MS` (**proposed: 30_000ms**,
  matching the agent-pane precedent) force a write even if edits keep arriving,
  so a continuously-fidgeting user still gets bounded crash-loss.
- **Serialize writers.** Adopt `useSnapshotPersistence.ts:51,73`'s
  `inFlightSnapshot` promise-chain equivalent so a debounced write and a
  close-time write cannot interleave and let the older one land last. This is a
  race that file's own comments call out by name; do not rediscover it.
- **Same storage slot.** Keep writing `Client.meta["session:last_topology"]`.
  No new table, no migration. `snapshot_workspace` already produces exactly the
  right value shape and already handles block-id placeholderization
  (`session_restore.rs:132,158`) so restored panes get fresh block ids.

**Explicitly unchanged:** the restore side. `restore_last_session`
(`session_restore.rs:286`) and its `clear_last_session_snapshot` consumption in
`window_create.rs:391` keep working as-is — they read the same key, and simply
find a fresher value.

### 4.2 Phase B — distinguishing a clean quit from a crash

Once the record survives a crash, restore has to answer a question it never
faced before: *was this snapshot left by a user who quit, or by a process that
died?*

They warrant different handling, and conflating them is how a restore feature
becomes annoying:

- **Clean quit** → restore silently. This is today's behavior and should stay
  invisible.
- **Crash / unclean exit** → restore, but say so. A small, dismissible
  indication that the workspace was recovered after an unexpected exit, rather
  than silently pretending nothing happened. This also gives the user a
  cheap escape hatch if the recovered state is itself implicated in the crash.

**Mechanism:** an explicit liveness marker, not inference. Write a
`session:clean_exit` boolean (or a monotonic `session:exit_seq`) alongside the
snapshot: cleared when srv starts, set only on a completed graceful shutdown.
Absent-or-false on next start ⇒ the prior run did not exit cleanly. This mirrors
the migration-marker reasoning already established in
`SPEC_MIGRATION_SYSTEM_HARDENING_2026_08_03.md` Phase 0a/0b — a marker's
*existence* is not proof of *effect*, so the marker must be written after the
work it attests to, not before.

### 4.3 Phase C — make the shutdown path worth trusting

Phase A's continuous writes reduce how much a broken shutdown costs, but they
do not excuse it. Three fixes, in ascending order of scope:

1. **Surface snapshot failure.** Have `save_last_session_snapshot` return a
   result the close handler can act on, and let the shutdown UI (§5) narrate a
   failed save instead of silently claiming success. Keep it non-blocking —
   never let a failed snapshot wedge a close — but stop lying about it.
2. **Close the host/srv timing race.** Give the host a real completion signal
   from srv rather than a fixed 2000ms guess. The countdown-modal spec
   identifies this same need ("a launcher-visible srv-completion
   acknowledgement, not designed here") and leaves it open; this spec claims it
   as in-scope, because "saved your session" is only honest if the save
   actually completed before exit.
3. **Give Windows a graceful backend shutdown** (issue #2979). Replace the
   unconditional `child.kill()` + `TerminateProcess` with a real request/ack +
   bounded-wait + escalate-to-kill sequence, matching the Unix path's shape.
   This is the largest piece and the one most worth splitting into its own PR.

## 5. Relationship to the shutdown countdown modal (#2981)

These two specs are complements, not alternatives, and should land in this
order: **this spec supplies the substance that spec's UI narrates.**

`SPEC_SHUTDOWN_COUNTDOWN_MODAL_2026_09_04.md` proposes a progress phase with
stages including *"Saving your session"*. Today that stage would narrate a
fire-and-forget write that may not have happened, on a platform (Windows) where
the backend is about to be hard-killed regardless. After Phase A + C it
describes something real: a record that is already near-current before shutdown
even begins, flushed once more with a completion signal the launcher can
actually wait on.

Recommended sequencing: **Phase 0 → Phase A → Phase C.1/C.2 → countdown modal → Phase
C.3**, so the modal is never in the position of narrating a stage that cannot
be trusted.

**Tray interaction (#2978,`SPEC_TRAY_OPTIONAL_BACKGROUND_SERVICE_2026_09_04.md`):**
if closing the last window becomes hide-to-tray rather than quit, the
close-time flush stops being the primary persistence trigger — which is an
argument *for* Phase A, not against it. Continuous writes are cadence-agnostic;
they do not care which action turns out to be "the last one."

## 6. Non-goals / explicit scope limits

### 6.1 What a session record is not

Only **topology** is captured — window set, tab structure, split/layout tree,
block view types, and the launch-relevant meta needed to recreate a pane
(agent identity, cwd). This spec does **not** extend the record to cover live
process state, terminal scrollback, or in-flight agent conversation content.
Those persist through their own separate, already-global mechanisms (agent
definitions, registry, and transcripts are global per `CLAUDE.md`), and pulling
them into a debounced session write would be both enormous and pointless.

Concretely: agent output arriving does **not** dirty the session record.

### 6.2 Out of scope entirely

- **Named saved Layouts** (Feature 2 of the 08-13 spec) — still unbuilt, still
  worth building, unrelated to persistence cadence.
- **Snapshot history / versioning.** `save_last_session_snapshot` overwrites in
  place and this spec keeps it that way. "Restore the session from two runs
  ago" is a different feature with different storage needs.
- **Cross-machine session sync.** The record stays local to its data dir.

## 7. Open questions

1. **Debounce and ceiling values.** Proposed 1000ms debounce / 30_000ms hard
   ceiling. The layout tree already uses 100ms and agent panes 30s; a session
   record is coarser than the former and cheaper than the latter. Worth
   measuring actual write cost against a large workspace before fixing these.
2. **Should Phase B's crash indication be a toast, a modal, or a passive
   marker?** A modal on every crash-recovery is likely too heavy; a silent
   restore loses the escape hatch. Leaning toast.
3. **Does a crash-recovered session restore agents automatically?** The 08-13
   spec's Feature 2 deliberately gates agent relaunch behind confirmation
   (citing Zellij's "press ENTER to run") to avoid silently re-spawning costly
   AI agents. Restore-on-relaunch today does relaunch. After a *crash*
   specifically, auto-relaunching an agent that may itself have caused the
   crash deserves a second look.
4. **Should `window:confirmclose` finally be wired up?** It exists in three
   places and is read by zero call sites. Both this spec and the countdown
   modal spec touch the close path; if it is ever going to become real, that is
   the moment.

## 8. Testing

- **Phase A**: unit-test the dirty-gating (topology mutations dirty the record;
  agent output does not), the debounce coalescing, and the hard-ceiling forced
  write. Integration-test that a snapshot exists mid-session, before any close.
- **Phase B**: kill srv mid-session (no graceful path) and assert next start
  both restores the workspace and reports it as crash-recovered; contrast with
  a clean quit restoring silently.
- **Phase C**: assert the host waits for a real srv completion signal rather
  than a fixed timeout; on Windows specifically, assert srv receives and
  completes a graceful shutdown request before the host exits.
- **Regression guard for the original bug class**: the 2026-07-16 retro exists
  because leaked window rows *looked* like a feature. Add a test that a clean
  quit leaves no `Window`/`Workspace` rows behind — restore must come from the
  snapshot, never from undeleted live state.
