# REPORT — the window is blank for seconds after the splash disappears, and none of that time is in the splash's own timeline

**Date:** 2026-09-03 (diagnosis), implemented and live-verified 2026-09-04
**Author:** Agent5
**Status:** implemented — both defects fixed on Windows in this repo's `main`
(commit history below). Diagnosis (§1-4) is unchanged from the original pass;
§5 is updated to describe what actually shipped, including one deliberate
divergence from the original proposed shape (§5.3).
**Platform:** Windows (reported, fixed, live-verified). macOS shares defect A
verbatim and is unfixed here by design — see §6. Linux already had the fix
for A before this report existed; it now also gets defect B's `paint` stage
for free, since the fix was implemented as a shared, not Windows-only, code
path.

---

## 1. The report, restated precisely

Two distinct complaints, and they turn out to be two distinct bugs that happen
to sit next to each other in the startup sequence:

1. **"The splash screen says things are done" — but the main window is then
   blank for a further stretch before anything real appears.** The splash's
   own stage list reaches its end and disappears; the window that replaces it
   shows nothing for seconds.
2. **The window should show fully painted, not spend seconds blank.**

(1) is a *measurement* gap — real work is happening, but the splash's timeline
doesn't know about it, so it reports "done" early. (2) is a *rendering* gap —
the window is being shown before it has anything to show. Both are real, and
both are verified against the current code below, not inferred.

---

## 2. Defect A — the window is shown before it has painted anything (Windows/macOS)

### 2.1 What actually gates `window.show()` today

`agentmux-cef/src/client/navigation.rs::on_load_end` — CEF's "main-frame HTML
finished loading" callback. For the top-level window, on every platform
**except Linux**, the code goes straight to:

```rust
window.show();
if let Some(b) = browser { if let Some(host) = b.host() { host.set_focus(1); } }
```

`on_load_end` is not "the page painted." It is document-load-complete, and the
code's own doc comments say so, in two places:

> `report_first_paint`'s doc comment: *"as opposed to CEF's `on_load_end` which
> only means 'main-frame HTML finished loading' and can fire before anything
> has visually painted."*

> `reveal_top_level_window`'s doc comment, describing the fix Linux got and
> Windows/macOS didn't: *"the real double-rAF signal for the main window
> landed ~2.08s after `on_load_end`."*

That 2.08s figure is not a worst case pulled from thin air — it's a measured
number from `docs/specs/REPORT_NEW_WINDOW_STARTUP_COLOR_FLASH_2026_07_14.md`'s
own verification run, on the same class of machine this investigation ran on
(a resource-shared dev box running several concurrent CEF instances). Also
measured that day: pool-window prewarms (an unrelated code path, so this is a
baseline reading of compositor-first-frame latency on that environment, not an
artifact of the gate) landed 1.36–1.84s after their own equivalent starting
point. **A one-to-two-second gap between "document loaded" and "something is
actually on screen" is the demonstrated norm on this class of hardware, not a
tail case.**

### 2.2 The fix already exists — for one platform

`reveal_top_level_window` (same file) has a real paint gate, but it is
`#[cfg(target_os = "linux")]`:

- The frontend fires a double-`requestAnimationFrame` at the very top of
  `bootstrap.ts` (`frontend/bootstrap.ts:40-46`) — the standard proxy for "the
  compositor actually presented a frame" — and reports it via the
  `report_first_paint` IPC command.
- On Linux, `on_load_end` **arms a gate instead of showing the window**, with a
  4000ms safety-net timeout in case the signal never arrives (crashed JS, rAF
  never firing). The real signal races the timeout; whichever comes first wins,
  once, per-arm (an `epoch` counter guards against a stale timeout firing after
  a reload re-armed the same label).
- `agentmux-cef/src/commands/backend.rs::report_first_paint`'s own doc comment
  states the gap outright: *"On Linux this unblocks the window `on_load_end`
  deferred... On other platforms it's currently just logged for telemetry —
  Windows/macOS aren't gated on it."*

So the signal exists, is already wired end-to-end from the frontend, is
already received by the host on every platform — and is thrown away on
Windows and macOS. This is the single largest, most directly actionable
finding in this report: **the fix for the exact bug reported is already built
and shipping on one platform.**

### 2.3 Why a click or a resize doesn't fix this one

Worth naming since it's easy to conflate with a different, already-fixed
issue from this same investigation session (the resize-then-drag report,
unrelated): this is not a focus/activation problem. The window is genuinely
empty pixels — nothing has been asked to paint yet — for the whole gap. No
user action shortens it; only time (and whatever the frontend is doing) does.

---

## 3. Defect B — the splash's timeline stops long before the app is actually usable

### 3.1 What the splash currently measures, exactly

Confirmed by reading every `stage_begin`/`stage_end` call site
(`agentmux-launcher/src/supervisor/windows.rs`, `agentmux-cef/src/lib.rs`) and
cross-checked against `docs/specs/PLAN_SPLASH_TELEMETRY_OPEN_ITEMS_2026_07_22.md`
(items #2 and #7, both explicitly verified there by exhaustive grep — this
report's own grep reproduces the same result independently):

| Stage key | Covers | Ends at |
|---|---|---|
| `migrations` | srv DB migrations | subprocess exit |
| `backend` | srv spawn | `AGENTMUXSRV-ESTART` on srv stderr |
| `host` | CEF host **process spawn** | `spawn_host_supervised` returning a live `Child` — **not** first paint. The comment at `windows.rs:453-461` says this explicitly: *"not full first-paint... Extending this stage to span to first-paint is a follow-up once a race-safe signal exists."* That signal (`report_first_paint`) now exists (§2.2) and was never plugged back into this stage. |
| `dlopen` | Loading `libcef`/`libcef.dll` | framework loaded |
| `cef_init` | `CefInitialize()` | returns |

**That is the complete list.** Nothing covers:

- Window creation → `on_load_end` (the load itself)
- `on_load_end` → real compositor first paint (defect A's own gap)
- Frontend bootstrap: `setupCefApi()` (an async IPC round-trip),
  `initApp()`, RPC/store hydration, tab/pane mount
- The tab-content reveal gate's own settle window
  (`frontend/app/store/tab-reveal.ts`) — up to `MAX_GATE_MS` (800ms) of
  intentional additional hold, waiting for a window of long-task-free frames
  after mount, before the in-page "brain" overlay (`#startup-loading`,
  `frontend/app/init/startup-splash.ts`) cross-fades out

Grep confirms zero frontend-side stage reporting exists at all:
`frontend/` has no reference to `sendLauncherMsg`, `startup_stage_begin`, or
`startup_stage_end` anywhere. The original telemetry spec
(`SPEC_SPLASH_STARTUP_TELEMETRY_2026_06_25.md`, Implementation Order step 7,
"Frontend events") was never built. `PLAN_SPLASH_TELEMETRY_OPEN_ITEMS_2026_07_22.md`
already reached this same conclusion in July and correctly logged it as an
open item rather than a rumor — this report adds nothing new on that point,
only confirms it still holds two months later and connects it to the live bug
report that makes it worth acting on now.

### 3.2 Consequently, two separate un-measured stretches are invisible

1. **The `on_load_end` → real-paint gap** (defect A, up to ~2s observed) — the
   splash has already declared `host` complete by the time this gap even
   starts, since `host` ends at process-spawn, long before `on_load_end`
   fires at all.
2. **The frontend-mount-and-settle gap** — everything from `setupCefApi()`
   through the tab-reveal gate's 80ms-clean-frames-or-800ms-cap settle
   detector. This is real, often substantial work (async IPC handshake,
   store hydration, tab restoration, first render, measured layout reflow)
   with zero telemetry representation.

The splash's own design already anticipates a live "running clock" row for
whatever is currently in flight (`SPEC_SPLASH_STARTUP_TELEMETRY_2026_06_25.md`
§"Running Clock") — the mechanism to show "still working, N seconds and
counting" exists. It simply has no stage to attach to for either of these two
gaps, so the splash instead shows nothing in-flight and looks finished.

### 3.3 Why this reads as "says done, but isn't" rather than "runs long"

The splash's summary hold (§"Summary Hold" in the June spec) is meant to fire
once real first-paint happens and hold briefly to let the user read the
timeline. Today, because nothing gates on real first-paint on Windows (defect
A), the hold logic has no correct trigger to attach to either — the closest
proxy available to it is `on_load_end`, the same too-early signal responsible
for the blank window in the first place.

---

## 4. These two defects compound, they don't just coexist

Fixing A alone (port the Linux paint gate to Windows/macOS) removes the blank
window, but the splash would still dismiss and the window would still appear
to "just be there" with no accounting of the wait — the user would see a
correctly-painted window arrive after an unexplained pause, rather than a
blank one. Fixing B alone (wire frontend + real-paint stages into the splash)
would make the *existing* too-early dismiss more informative but wouldn't stop
the window itself from flashing blank first — the timeline would show
"Frontend init ▶ 1.8s..." running *in the splash*, while the *separate* CEF
window behind it still blanks out before that row even finishes. **Both need
fixing for the reported experience — "splash says done, then blank window,
then more loading" — to actually become "splash accurately shows the whole
wait, then the window arrives already painted."**

---

## 5. What shipped

Ordered by dependency, matching how the existing code was already structured
to receive each piece. §5.1 matches the original proposal. §5.2 diverges from
the original two-change proposal (§5.2+§5.3 in the first draft of this
report) deliberately, once implementing §5.1 first made a cleaner shape
visible — see §5.2's own explanation. §5.3 remains unimplemented, unchanged
from the original diagnosis.

### 5.1 Port the Linux paint gate to Windows (defect A) — the core fix

`reveal_top_level_window` isolated the platform-specific behavior behind a
single `#[cfg(target_os = "linux")]` block with a documented fallback. That
gate is now `#[cfg(any(target_os = "linux", target_os = "windows"))]` —
Windows shares the exact same state machine and functions Linux already used
(`reveal_gated_window`, `handle_first_paint_signal`, `PaintGateRevealTask`,
`FirstPaintSignalTask`, `on_frontend_first_paint`, `PAINT_GATE_NEXT_EPOCH`),
not a parallel reimplementation. `linux_paint_gate_pending`/
`linux_first_paint_seen` (`state/mod.rs`) keep their names — renaming them
would have touched every call site for no functional gain — but their doc
comments now say plainly that Windows populates them too.

What actually differs per platform is only the two calling-convention pieces
`reveal_top_level_window`'s own doc comment already flagged as the seam:

- **Dismiss signal.** Windows' `AGENTMUX_SPLASH_EVENT` `SetEvent` used to fire
  unconditionally at `on_load_end`, before anything had painted — the same
  bug in miniature. It now fires from inside `reveal_gated_window`, at the
  exact moment the gate resolves, right alongside the macOS/Linux
  ready-file write it sits next to in that function. This also closes a
  latent bug the move inherits a fix for: the old call had no
  `is_pool_window` guard (that check happens later in `on_load_end`), so a
  hidden pool-window prewarm racing ahead of the real main window during a
  cold start could dismiss the splash before the real window had loaded at
  all — precisely the class of bug Linux's ready-file placement was already
  immune to, for the identical reason cited in its own comment.
- **Safety-timeout value.** Reused Linux's `4000ms` verbatim rather than
  picking a separate Windows number. The comment on
  `PAINT_GATE_SAFETY_TIMEOUT_MS` now says why directly: this is a backstop
  against a genuinely stalled renderer, not a target, so the real signal is
  expected to win on any machine capable of running the app at all — and the
  live Windows run below confirms exactly that (`reason="signal"`, nowhere
  near the timeout).

**Live-verified**, not just compiled: a fresh `task dev` cold start on this
machine, host log (`agentmux-host-v0.55.34.log.2026-09-04`):

```json
{"message":"[startup-paint] frontend reported first paint","label":"main"}
{"message":"[startup-paint] revealing gated window","label":"main","reason":"signal","elapsed_ms":1177}
```

`reason="signal"` — the real double-rAF paint confirmation resolved the gate,
not the timeout. `elapsed_ms=1177` is the main window's `on_load_end`-to-
real-paint gap on this run: **over a full second that the old code would have
shown as a blank window**, and that now stays hidden behind the (already-
painted, non-blank) native splash instead. This is the same order of
magnitude as the Linux measurement in §2.1 (1.36-2.08s on a similar class of
machine), which is itself the strongest available confirmation that this
isn't a Linux-specific quirk — the gap is real on Windows too, and the fix
closes it the same way.

Pool-window prewarms (`floating-pool-...`, `window-pool-...`) also logged
`report_first_paint`, correctly with **no** matching `revealing gated window`
line — confirming they still skip the whole reveal path as designed (§2.2's
pool-window carve-out), rather than the fix accidentally widening its own
scope to windows that were never meant to go through it.

### 5.2 "paint" stage telemetry (defect B) — one new stage, not two stage changes

The original proposal (§5.2/§5.3 in the first draft of this report) was to
add a `frontend` stage plus extend `host` to span to first-paint. Implementing
§5.1 first made a cleaner shape visible, so this diverges from that proposal
deliberately:

**What shipped instead: one new stage, `paint`**, begun in
`reveal_top_level_window` at the exact moment the gate arms (i.e. where
`on_load_end` used to just call `window.show()`), and ended in
`reveal_gated_window` at the exact moment the gate resolves — the same two
places already carrying this report's core logic, via
`launcher_ipc::report_startup_stage_begin`/`report_startup_stage_end`, the
identical fire-and-forget API `dlopen`/`cef_init` already use successfully
(confirmed safe to call unconditionally: it's a no-op sender-side when no
launcher connection exists, e.g. in `task dev`).

Why this is a better fit than the original two-change proposal, not just a
smaller one:

- `host`'s own comment already defines it as *process-spawn* latency — a
  genuinely distinct, useful number ("how fast did the OS launch our
  process"). Extending it to also mean "...and also how long until paint"
  would have made one row answer two different questions.
- A separate `frontend` stage measured from the top of `bootstrap()` to the
  same first-paint signal would have mostly **duplicated** `paint`'s own
  span — `bootstrap()` starts essentially when `on_load_end` fires (the
  renderer's own script can't run meaningfully earlier), so a second,
  differently-named row ending at the identical instant would have added
  splash noise without adding information.
- No frontend/`bootstrap.ts` changes were needed at all: `paint`'s begin fires
  host-side (at `on_load_end`), and its end reuses the *already-existing*
  `report_first_paint` signal `reveal_gated_window` was already consuming for
  the reveal itself. One signal, already flowing, now feeding two consumers
  (the gate AND the stage row) instead of needing a new one invented for
  telemetry alone.

`apply_event`'s stage-row handling in `splash.rs` is fully key-driven — new
stage keys need no launcher-side changes, confirmed by `dlopen`/`cef_init`
already working this way. Nothing in `agentmux-launcher` was touched for this
half of the fix.

Not observed in the live `task dev` run above: `report_startup_stage_begin`/
`_end` log at `tracing::debug!`, and `task dev` doesn't connect to a launcher
IPC socket at all in dev mode (`should_connect_launcher` gates on
`!is_dev_build_exe`), so the calls fire as designed no-ops there — expected,
not a gap. Confirming the `paint` row actually renders in a real native
splash needs a packaged-build run; this pass's packaged-build smoke test
didn't produce a distinguishable separate window (a single-instance
collision with another same-day build already running on this shared,
multi-agent dev machine) and wasn't pursued further rather than risk
touching another agent's running instance. The core mechanism this stage
reports on is verified either way (§5.1); what's unverified is only the
final-mile "does the pixel row draw," which is the same class of gap
`dlopen`/`cef_init` already closed successfully using the identical API.

### 5.3 Frontend bootstrap / tab-reveal settle window — deliberately still not counted

Still open, unchanged from the original diagnosis. This is a judgment call,
not an engineering blocker, flagged rather than resolved here. Arguments both
ways:

- **For:** it's real, sometimes-substantial time the user is waiting with
  nothing telling them why (the in-page brain overlay is animated, but gives
  no duration/stage information the way the splash does).
- **Against:** the brain overlay already exists specifically to cover this
  exact window (`startup-splash.ts`'s own doc comment: *"must stay up...
  until the content-reveal gate decides the window has settled"*) — it may be
  intentional that this phase has its own, different-looking "still working"
  indicator rather than extending the native splash's lifetime further. Doing
  so would also mean holding the OS-native splash window open across the
  hand-off to the in-page one, which is a real architectural change, not a
  telemetry-only one.

---

## 6. What this fix does not cover

- **§5.3 (frontend bootstrap / tab-reveal settle window) is not implemented.**
  Deliberately: the brain overlay already covers it visually, and folding it
  into the native splash's own lifetime would be an architectural change, not
  a telemetry addition — see §5.3's own for/against.
- **Windows-specific safety-timeout tuning was not done.** `PAINT_GATE_SAFETY_TIMEOUT_MS`
  reuses Linux's `4000ms` verbatim (§5.1's own reasoning for why that's
  the correct conservative choice, not a placeholder). If it's ever observed
  firing on real Windows hardware outside a genuinely crashed renderer,
  that's the signal a dedicated measurement pass is now overdue — the
  comment on the constant says this explicitly.
- **The `paint` splash row's actual on-screen rendering was not visually
  confirmed** — §5.2's live run used `task dev`, which never connects to a
  launcher IPC socket, so the `report_startup_stage_begin`/`_end` calls
  fired as designed no-ops there. The underlying reveal mechanism they
  report on IS confirmed live (§5.1's `elapsed_ms=1177` evidence); a
  packaged-build re-run to watch the row itself draw is the natural
  follow-up, not blocking given `dlopen`/`cef_init` already prove the same
  API renders correctly.
- **macOS is untouched**, deliberately. `reveal_top_level_window`'s cfg gate
  is `any(linux, windows)`, not widened further — macOS keeps its exact
  pre-existing behavior (unconditional `window.show()` at `on_load_end`),
  matching this report's own scope statement in the original diagnosis
  pass and the "don't touch what you can't verify" discipline
  `PLAN_SPLASH_TELEMETRY_OPEN_ITEMS_2026_07_22.md` already established for
  the reverse case (a macOS-only environment declining to touch Windows
  code blind). Defect A's applicability to macOS is read from the code
  (identical unconditional fallback), not separately measured; defect B's
  `paint` stage is Windows/Linux-only by the same cfg gate, so macOS's
  splash timeline is unchanged by this pass.
- `PLAN_SPLASH_TELEMETRY_OPEN_ITEMS_2026_07_22.md` items #4 (Windows
  pre-supervisor "prep" stage — splash doesn't exist yet at that point on
  Windows, a structural gap), #5 (cross-platform `StageRow` consolidation),
  #6 (IPC-signal dismiss instead of file-poll, macOS-specific), and #8 (full
  srv/host spawn parallelization) remain real, already-tracked, and out of
  scope here — none of them are on the path between `on_load_end` and first
  paint, which is where this report's two defects lived.

---

## 7. References

- `agentmux-cef/src/client/navigation.rs` — `on_load_end`,
  `reveal_top_level_window`, `reveal_gated_window`, `handle_first_paint_signal`
- `agentmux-cef/src/commands/backend.rs::report_first_paint`
- `frontend/bootstrap.ts:20-46` — the double-rAF signal
- `frontend/app/init/startup-splash.ts`, `frontend/app/store/tab-reveal.ts`
- `agentmux-launcher/src/supervisor/windows.rs:258-300,450-490` — splash spawn,
  `host` stage bounds
- `agentmux-cef/src/lib.rs:396-415,714-723,1055-1056` — `dlopen`/`cef_init`
  stage reporting
- `docs/specs/SPEC_SPLASH_STARTUP_TELEMETRY_2026_06_25.md` — original design,
  step 7 ("Frontend changes") never implemented
- `docs/specs/PLAN_SPLASH_TELEMETRY_OPEN_ITEMS_2026_07_22.md` — items #2, #4,
  #7 independently confirm §3's findings from a prior, separate pass
- `docs/specs/REPORT_NEW_WINDOW_STARTUP_COLOR_FLASH_2026_07_14.md` — the
  Linux investigation and fix defect A needed ported (§5.1: now done)
- `agentmux-cef/src/state/mod.rs` — `linux_paint_gate_pending`/
  `linux_first_paint_seen`, doc comments updated to say Windows populates
  them too (names kept to avoid an unrelated-diff rename)
- Live evidence (§5.1): `agentmux-host-v0.55.34.log.2026-09-04`, a fresh
  `task dev` cold start on this machine, 2026-09-04
