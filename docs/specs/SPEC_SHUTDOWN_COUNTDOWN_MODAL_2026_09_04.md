# A formal shutdown sequence: countdown-confirm modal + splash-style progress

**Status:** proposed
**Date:** 2026-09-04
**Author:** Manoz@Area54
**Motivating observation:** repo owner, live session — closing AgentMux
today visibly "does a couple things" with no explanation, unlike startup,
which has a splash screen narrating its own stages. Ask: a modal with a 5s
countdown, "Close now" / "Cancel", where the countdown elapsing is what
actually starts shutdown; and progress narration during the teardown that
follows, mirroring the startup splash.

---

## 0. The headline finding: shutdown already has no visible ceiling

This isn't cosmetic. Today, on Windows, a wedged host with zero open
windows can sit "quitting" for **up to ~30–150 seconds** with **no UI at
all**, before a backstop force-kills it (`teardown_backstop.rs`'s
`TEARDOWN_GRACE` = 30s, plus up to `WATCHDOG_LAG_RETRIES_MAX` (3) × 3s
watchdog-lag retries — worst case ≈ grace + two 60s-spaced UI probes).
Separately, the *normal* close path already does real, sometimes-unbounded
work with no reporting: a pool-window close cascade, a 2s-timeout TCP round
trip to srv, a session-restore snapshot write, a workspace-delete saga, and
5s-per-shell SIGTERM→SIGKILL graces. None of it is narrated anywhere.

So the countdown/progress modal isn't just UX polish on top of a fast,
already-understood path — it's the first user-visible acknowledgment that
shutdown can take real, variable time, and the first thing that would let a
wedged shutdown be told apart from a fast one.

## 1. Current shutdown, condensed (full trace with citations is the
research record behind this spec; key facts only, here)

- **No quit menu item, no OS shutdown/logoff handler, no `SetConsoleCtrlHandler`, anywhere.** The only real trigger is the user closing the last top-level window (`WM_CLOSE` → CEF `do_close` → `on_before_close`, `agentmux-cef/src/client/lifecycle.rs:538,893`). Unix signals (SIGINT/TERM/HUP) reach the **launcher**; Windows has **no signal arm at all** (`agentmux-launcher/src/supervisor/windows.rs:799-800` — shutdown flows via host/srv only).
- **`on_before_close` alone does real synchronous work** before anything is torn down: cascade-closes OAuth popups, evicts window/floater state, cancels parked credential approvals, reports to the launcher — then Stage 1 posts `WM_CLOSE` to every warm-pool browser (`ui_tasks/window.rs:589-660`), each running its own `on_before_close`.
- **`backend_close_window`** opens a raw TCP socket to srv with a **2000ms** timeout (`client/helpers.rs:100-104`), and only *after* that thread finishes does the host post `QuitMessageLoopTask` back to the UI thread (the 2026-07-16 last-window-close race fix).
- **srv's `window.CloseWindow`** does the heavy lifting on the final close: saves a session-restore snapshot (`window_close.rs:222-232`), then runs the `delete_workspace` saga, which kills every block's PTY controller.
- **Host exit is a hard self-terminate on Windows**: `child.kill()`s the srv sidecar (no SIGTERM, no graceful RPC), then `TerminateProcess(GetCurrentProcess(), 0)` (`agentmux-cef/src/lib.rs:1246,1293-1297` — deliberate, to dodge a CEF DLL_PROCESS_DETACH false-crash signal). **Unix is genuinely graceful in the same spot**: SIGTERM to host and srv, `tokio::time::timeout(1500ms)`, then SIGKILL on survivors (`supervisor/unix.rs:739-760`).
- **Per-shell teardown has its own 5s grace** (`KILL_GRACE_SECS`, `blockcontroller/shell/controller.rs:42-44`); srv's own final `stop_all()` has an **800ms** flat grace (`agentmux-srv/src/main.rs:201-205`) — but that only runs on the Unix stdin-EOF/signal path. **On Windows, srv is never asked to shut down gracefully at all.**
- **No quit confirmation exists.** `window:confirmclose` is declared in three places (`wconfig/types.rs:179-180`, `schema/settings.json:162-164`, `gotypes.d.ts:1760`) and read by **zero** call sites. `beforeunload` in the frontend only calls `dispose()` — no prompt (`app-init.ts:893`; the comment there says the real cleanup was deliberately moved into the host's `on_before_close`, replacing this).

## 2. Architecture decision: launcher-owned native modal, not a frontend one

This is the load-bearing decision and it's already answered by precedent,
not invented here.

**The startup splash is a real, separate, native OS window owned by the
LAUNCHER** — Win32 layered popup on Windows (`agentmux-launcher/src/splash.rs`,
software-drawn DIB, zero GPU/Chromium), Cocoa `NSWindow` on macOS
(`splash_mac.rs`), x11rb/wayland on Linux (`splash_linux/`). It exists in
the launcher specifically because *"the thing it's covering doesn't exist
yet"* (`main.rs:111-118`'s own comment) — the host/renderer aren't up.

**Shutdown has the mirror-image version of the same problem**: the thing
you'd want to render the modal (the host's Chromium renderer) is the thing
being torn down, and on Windows it doesn't get a teardown callback at all —
the process just calls `TerminateProcess` on itself. A frontend modal would
be destroyed mid-shutdown, potentially before the teardown it's narrating
even finishes. `SPEC_GRACEFUL_OOM_EXIT_2026_06_29.md` §2 already made this
exact argument from the OOM-exit angle and reached the same conclusion:
*"the launcher — not the dying host — should own the 'explain + verify
cleanup' step."*

**So: the shutdown modal is launcher-owned and native, using the same
mechanism as the splash — not the frontend `modalmodel.ts`/
`modalsrenderer.tsx` system**, which requires a live, responsive renderer
and is torn down early in the sequence (Stage 1's pool-window `WM_CLOSE`
cascade fires before the srv round trip even starts).

### Reusing the splash's actual infrastructure

- `spawn_splash` is **already proven re-invocable mid-session** with a
  fresh channel and a fresh uniquely-named event — that's exactly what
  `respawn_splash_for_restart` (`supervisor/windows.rs:17-54`) does for
  crash restarts, including a sequence counter guaranteeing name uniqueness.
  The shutdown modal reuses this, not a new spawn mechanism.
- The status channel is an **in-process `std::sync::mpsc` of `StartupEvent`**
  (`StageBegin`/`StageEnd`/`SubBegin`/`SubEnd`, `startup_events.rs:23-51`),
  fed by both the launcher directly and the host over IPC
  (`launcher_ipc::report_startup_stage_begin/_end`). This is a different,
  narrower channel than the `[launcher-event]` JS bridge
  (`launcher_event_bridge.rs`) that already carries `host_should_quit` today
  — that bridge needs a live renderer and currently drives **no UI at all**
  for that event (`launcher-event/reducer.ts:63-74` just logs it). The
  splash-style channel is the right one to extend; the JS bridge is not.
- `apply_event` (the splash's rendering logic, all three platforms) is
  **fully key-driven** — new stage keys render with no renderer-side change,
  confirmed by the recent `dlopen`/`cef_init`/`paint` additions. Shutdown
  stages are new keys on the same event shape, not a new render path.
- **The most direct precedent is already merged**: #2967's long-startup
  reassurance (`supervisor/windows.rs:479-533`) is a launcher-side timer,
  gated on a real state check (not elapsed time alone), emitting a
  `SubBegin` sub-row with a live counter and deliberately generic wording.
  That exact pattern — *"the launcher reports on the thing because the
  thing can't report on itself"* — is arguably even more true for shutdown
  than for startup.

### What this requires that doesn't exist yet

The trigger today is host-side and immediate: `do_close` → straight into
teardown. For a launcher-owned modal with a real Cancel, the host needs to
**defer** the close and ask the launcher first:

1. `do_close` sends a new IPC message (`quit_requested`) to the launcher
   **instead of proceeding**, and returns `true` from the CEF callback
   (standard CEF pattern: `true` suppresses the default close, the app
   closes for real later via an explicit browser-close call once granted).
2. Launcher spawns the shutdown modal via `spawn_splash`'s reuse path,
   starting in **countdown phase**: "Shutting down in 5…", Close now /
   Cancel.
3. **Cancel** → launcher IPCs `quit_cancelled` back to the host; host does
   not proceed with `on_before_close`'s teardown; the modal dismisses;
   nothing else changes. This is the first real implementation of
   `window:confirmclose`'s spirit, though not its literal settings key —
   see §6.
4. **Close now**, or the countdown reaching 0, → launcher IPCs
   `quit_confirmed`; host resumes the exact existing teardown sequence
   (§1's steps A–G, unchanged), and **switches the modal from countdown
   phase to progress phase** — same window, same channel, `StageBegin`/
   `StageEnd` rows narrating the real steps as they happen (window-close
   cascade, srv notify, session snapshot, agent shutdown), exactly the way
   the splash narrates `dlopen`/`cef_init`/`paint` today.
5. Modal dismisses when the launcher observes host exit — it already does
   this today (`supervisor/windows.rs:721-741`); no new detection needed.

## 3. What actually gets narrated (the honest version)

Reuse the real stages already identified in the shutdown trace, not
invented placeholder text — the same discipline #2967 used ("First run can
take longer" is deliberately generic because the real cause, Defender's
cloud reputation check, isn't knowable from that code path):

| Stage | Source | Bound worth showing |
|---|---|---|
| Closing windows | pool-window `WM_CLOSE` cascade | unbounded (N-dependent) — show a running count if cheap |
| Saving your session | `session_restore::snapshot_workspace` | unbounded SQLite write, normally fast |
| Stopping agents | `delete_workspace` saga → PTY controller kills | up to 5s per shell (`KILL_GRACE_SECS`) |
| Notifying background service | `backend_close_window` TCP round trip | 2000ms timeout |

If the wedged-host backstop (`TEARDOWN_GRACE` = 30s) or the quit watchdog
(3s × up to 3 retries) engage, that's the moment to say something different
— "This is taking longer than expected" — rather than let the countdown
UI imply everything is proceeding normally for 30–150s. This is the same
"reassurance on a real threshold, not a guess" pattern as #2967.

## 4. Cross-platform sequencing

Ship the same order the splash itself shipped in: **Windows first**. Reasons
specific to this feature, not just convention:

- macOS shutdown is *already* the graceful one (SIGTERM + 1500ms wait) —
  the narrative gap is real but the underlying mechanism doesn't need the
  parallel fix §6 describes.
- macOS's `Splash::show()` **parks the main thread forever**
  (`main.rs:135,155`) — a shutdown-phase reuse of the same window needs a
  different threading answer there (the supervisor already runs on a worker
  thread and calls `std::process::exit`, so "reuse the same window" isn't a
  drop-in on macOS the way it is on Windows).
- Linux's splash polish (fade-out, positioning) is itself still
  **Draft/in-progress** per `SPEC_LINUX_SPLASH_POLISH_2026_06_20.md` — the
  shutdown modal would inherit whatever visual gaps that leaves.
- Wayland can't be positioned or forced on-top at all (protocol
  limitation) — worth deciding whether a best-effort, non-topmost shutdown
  modal is acceptable there or whether it's Windows/macOS/X11 only for v1.

## 5. Countdown and button semantics, precisely

- **On open:** countdown starts immediately at 5s. No auto-dismiss-and-hide
  on hover/focus-loss (unlike `SPEC_ASK_USER_QUESTION_TIMEOUT_HOVER_PAUSE_2026_08_10.md`'s
  pattern) — a shutdown countdown pausing because the user's mouse drifted
  over it would be surprising, not helpful. It's a countdown to a default
  action (proceed), not a question with a recommended answer.
- **"Cancel"** aborts the *close attempt*, not a value. The user keeps
  working; nothing has been torn down. This is the only place `Cancel` can
  be fully honest — once phase 2 (progress) begins, per §1's own trace,
  windows are already closing and PTYs are already dying; there is no clean
  "cancel mid-teardown" to offer. **Cancel is only available during the
  countdown phase**, not during progress.
- **"Close now"** skips the remaining countdown and immediately sends
  `quit_confirmed` — functionally identical to the countdown reaching 0,
  just sooner. Same code path, so there's no second teardown implementation
  to keep in sync.
- **Countdown reaching 0** sends the identical `quit_confirmed` message.

## 6. A prerequisite worth naming: Windows has no graceful backend shutdown at all

If the progress phase says "Stopping agents" or "Saving your session" on
Windows, that claim should be true. Today it isn't fully: srv is killed
outright (`child.kill()`, no SIGTERM, no graceful RPC) and its own 800ms
shell-stop grace only ever runs on the Unix stdin-EOF/signal path. This
spec doesn't require fixing that first, but flags it: narrating "stopping
your agents cleanly" over a path that's actually a hard kill is worse than
narrating nothing, because it's a claim the modal itself would be making
falsely. Either (a) give Windows an equivalent graceful-stop signal to srv
before the hard kill, gated by a short timeout, or (b) word the Windows
progress stage to match what actually happens ("Stopping AgentMux") rather
than implying a save-your-work guarantee it doesn't provide.

The same gap exists one level down, and more generally: `BlockController::stop()`'s
Unix SIGTERM+grace path (`shell/lifecycle.rs:886-894`) has no Windows
counterpart at all — Windows block teardown is unconditionally
`TerminateJobObject` (`process_tracker/windows.rs:203-221`), with zero grace,
for every block stop (pane close, tab close, workspace delete), not just app
shutdown. Tracked separately as
[agentmuxai/agentmux#2979](https://github.com/agentmuxai/agentmux/issues/2979)
since it's a block-level gap independent of this spec's scope, not something
to fix as part of the shutdown modal itself — but it's the same underlying
claim this section is about: "stopping your agents cleanly" isn't true on
Windows today at any level, sidecar or individual agent process.

## 7. Interaction with the proposed tray/background-service feature (PR #2978)

`SPEC_TRAY_OPTIONAL_BACKGROUND_SERVICE_2026_09_04.md` (Agent5, docs-only,
proposed) changes the meaning of this spec's own trigger condition, and the
two specs need to agree on that before either implements against `do_close`.

That spec states its load-bearing required change plainly: *"closing the
last window must stop meaning 'shut down the tree.'"* Its whole premise is
that once a tray icon exists, closing the last window should default to
**hiding to tray** (srv + launcher stay resident), not quitting. This
spec's entire design, by contrast, is anchored on "user closes the last
window" as the quit trigger that opens the countdown modal (§2, step 1).
If tray ships as designed, that trigger fires on the wrong action —
window-close would no longer mean "the user wants to quit" at all.

**Resolution this spec adopts:** the two features are compatible, but the
trigger point needs an explicit split, not a shared one:

- **Without tray** (today, and the interim state while tray is unbuilt):
  window-close remains the only quit signal, and this spec's design
  applies unchanged — the countdown modal fires on `do_close` exactly as
  described in §2.
- **With tray** (if/when #2978 ships): window-close instead triggers the
  tray's hide-to-pool behavior by default; the countdown modal described
  here should fire on an explicit **"Quit AgentMux"** action instead (a
  tray context-menu item, or an equivalent explicit affordance) — not on
  ordinary window-close. The `do_close`-deferral IPC handshake this spec
  proposes (§2) still applies to that explicit quit action; only the
  *event* that triggers it changes.

**Why this doesn't block either spec today:** #2978 is proposed/design-only
with no prototype, and its own phased rollout (§7 there) puts the
clean-exit/tray-teardown decoupling as step 1, *before* any tray icon
exists — meaning "window-close = quit" stays true for a while yet even if
that spec is approved. This spec's `do_close`-defer mechanism (§2) is
useful and buildable independent of tray: it's the same interception point
either feature needs, just wired to a different upstream trigger. Whoever
implements either feature first should leave a comment at the `do_close`
call site cross-referencing the other spec, so the second implementation
doesn't have to rediscover this coupling.

## 8. Non-goals

- **Not** a `window:confirmclose`-settings-driven optional prompt. That key
  exists and is unused; this spec doesn't wire it up as a togglable
  preference — the countdown-modal behavior described here would be the
  new unconditional default. Whether it should also be user-disable-able is
  an open question (§10), not assumed here.
- **Not** a cancel path once teardown has genuinely started (§5). Framing a
  mid-teardown abort as achievable would misrepresent what's actually
  reversible.
- **Not** an OS shutdown/logoff handler (`WM_QUERYENDSESSION` etc.) — none
  exists today (§1) and adding one is a separable, larger change (the OS
  gives very little time to respond to a logoff, which conflicts directly
  with a 5s user-facing countdown). Worth its own spec if wanted.
- **Not** fixing the Windows graceful-backend-shutdown gap (§6) — flagged
  as a prerequisite-or-honesty-tradeoff, not built here.
- **Not** a redesign of the teardown sequence itself (§1's steps A–G stay
  exactly as they are). This spec adds narration and a confirm gate in
  front of the existing sequence, not changes to the sequence.

## 9. Testing

- Launcher-side: unit tests on the new `quit_requested`/`quit_cancelled`/
  `quit_confirmed` IPC state machine, mirroring the existing
  `respawn_splash_for_restart` test coverage pattern.
- A genuine Cancel test: trigger close, send Cancel before the countdown
  elapses, assert the host never proceeds into `on_before_close`'s teardown
  cascade and the window remains fully interactive.
- A genuine Close-now test: assert it produces the identical teardown
  sequence as countdown-elapsed, not a separate fast path.
- Visual verification is the same known gap the splash itself has today
  (`REPORT_SPLASH_TO_FIRST_PAINT_BLANK_WINDOW_GAP_2026_09_03.md` notes the
  `paint` stage row was never visually confirmed on real hardware, since
  `task dev` doesn't connect a launcher IPC socket) — this needs the same
  real-machine, packaged-build verification step, not just unit tests.

## 10. Open questions for the repo owner

1. **Is the countdown-confirm gate unconditional, or should
   `window:confirmclose` become the real toggle for it** (default on,
   user-disableable)? The key already exists and does nothing; this is the
   first proposal to give it a job.
2. **Cross-platform v1 scope** (§4) — Windows-only first, matching how the
   splash itself shipped? Or Windows + macOS together, skipping Linux until
   its own splash polish work (`SPEC_LINUX_SPLASH_POLISH_2026_06_20.md`)
   lands?
3. **§6's Windows graceful-shutdown gap** — fix it as a companion PR before
   this ships, or ship this first with honestly-worded Windows narration
   and fix §6 separately?
4. Should the progress phase show a **window count** during the pool-close
   cascade (real, if cheap to read), or keep it to named stages only, to
   avoid a number that's noisy for anyone with many tabs open?
