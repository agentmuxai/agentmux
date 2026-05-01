# Tab Tear-Off — Chrome-Faithful Window-Move Architecture

**Date:** 2026-04-26 (rewritten to drop the canvas-ghost approach;
           re-revised same day to raise the quality bar)
**Status:** Spec only (Phase 1 threshold detection shipped in PR #559;
           Phases 2-7 not started — **next priority** post Phase E.5.)
**State-correctness foundation:** Phase E.5 (PRs #619-#622) routed every
  workspace/tab/block/window mutation through the srv reducer + sagas.
  When this spec's Phases 2-7 land, the SC_MOVE-driven flow dispatches
  the existing `TearOffTabSaga` / `TearOffBlockSaga` / `RestoreTornOffTabSaga`
  / `MoveTabToWorkspace` reducer command — those don't change with the
  UX rewrite. Reducer + sagas also subsume the "merge" + "cancel-back"
  state transitions described in Phases 4-5 below.
**Smoke status (2026-05-01):** the current half-HTML5 / half-SC_MOVE
  hybrid was smoke-tested and surfaced a "no drop zone on reconnect"
  symptom. Diagnosis halted — the offending code path
  (`start_cross_drag` / `update_cross_drag` / `complete_cross_drag` +
  `cross-drag-update` / `cross-drag-end` events) is the code this spec
  replaces wholesale. Fixing under the current pipeline is throwaway
  work. See `docs/retro/phase-e-status-2026-05-01.md` §11.
**Trigger:** User feedback —
  > *"chrome tabs don't ghost, you just drag it out, and the
  > entire non-faded window drags around"*

  > *"can we replicate that exactly?"*

  > *"we want a solid solution, high performance, robust experience
  > of a full tear off just like chrome. time/expense is not a
  > factor … quality above all"*
**Scope:** Cross-window tab tear-off using Win32 `SC_MOVE` modal
           loop, the same mechanism Chrome / Edge / Arc use, with
           parity coverage for macOS and Linux.
**Owner:** TBD
**Supersedes:** the canvas-ghost approach in this file's first
           draft (commit history available via git).

---

## 0. Quality bar (read this before anything below)

This is **not** a feature where "good enough" ships. The user has
explicitly framed time/expense as not a constraint and demanded a
solid, high-performance, robust experience that matches Chrome.
Read every section through that lens:

- **No "MVP cuts."** Every phase below ships in full. No
  deferring the warm-window pool, no dropping cancel-back, no
  punting platform parity.
- **No "fallback to the existing dragend tear-off."** The current
  HTML5-dragend pipeline (`[dnd:cef] start_cross_drag` →
  `tear_off_tab` → `open_window_at_position` at end of drag) is
  the path this spec **replaces**. The old path is *deleted only
  once the new flow has full feature parity* — meaning Phases
  2 + 4 + 5 (handshake + merge + cancel) are landed and pass
  validation. Removing it earlier would regress merge / cancel
  behaviour. After parity: the new Chrome-faithful flow is the
  *only* code path on Win32 (and macOS / Linux/X11); no A/B
  switch, no settings toggle. Wayland keeps the dragend fallback
  because Wayland forbids the global cursor tracking the new
  flow needs (§7).
- **No "best-effort Linux."** macOS and Linux must reach
  perceptual parity with Win32 by ship time. Wayland's loss of
  global cursor tracking is the one acknowledged limitation
  (§7); everything else is at parity.
- **First-paint flash budget: 0 ms.** A pre-warmed window pool is
  mandatory (§4.5). The cold path exists only as a defensive
  guard against pool exhaustion and triggers a `WARN`-level log
  + telemetry event when it does, because that should never
  happen in practice.
- **Handshake timing budget: ≤ 8 ms** (one half-frame at 60 Hz)
  from threshold-cross to `SC_MOVE` posted. Chrome empirically
  sits in the 5-8 ms range; we match.
- **Cancel-back-to-source restores the EXACT original tab
  position.** Not "appended" to the strip — the source remembers
  the original index and reinserts there.
- **Cross-window merge insertion is pixel-accurate.** The merge
  insertion index is computed from the cursor's X coordinate
  against the destination strip's tab-bar geometry, the same way
  in-window reorder works. No "drop at end" shortcut.
- **Visible regressions count as bugs.** First-paint flash, tab
  duplication, ghost windows that don't close, "I let go and
  nothing happened" — every one is a P1 fix before merge.
- **Observability is part of the deliverable** (§10). Tear-off
  latency, merge success / cancel rate, pool exhaustion events,
  hook-install failures — all emit structured logs and counters.

If a tradeoff would compromise any of the above, escalate to the
user before taking it. Don't optimise for shipping the spec;
optimise for shipping the experience.

---

## 1. The behaviour we are replicating

Chrome's tab tear-off, end-to-end:

1. User mousedowns on a tab and starts dragging.
2. While the cursor stays inside the tab strip → tabs reorder.
3. The moment the cursor passes a vertical threshold (typically
   the bottom edge of the strip), the tab is **torn off**:
   - The tab vanishes from the source window's strip instantly.
   - A real, full-fidelity OS window materialises at the cursor
     position, already containing the tab's pane content.
   - That new window enters Win32's built-in modal window-move
     loop, so it follows the cursor at full opacity, no fade,
     no ghost.
4. While the user moves, Chrome watches the cursor for any other
   Chrome window's tab strip underneath.
5. On mouseup:
   - Cursor over another window's strip → the dragged window's
     tab is **merged** into that window (dragged window is
     destroyed; tab inserted at the cursor's X position in the
     destination strip).
   - Cursor anywhere else → the dragged window simply stays where
     dropped. No-op finalize.
6. If the user releases back over the source strip without ever
   leaving it → the tear-off is undone (or never started — see
   §4.1 threshold).

There is no HTML5 drag, no `setDragImage`, no transparent
overlay, no canvas. It is a sequence of (a) "spawn a window with
this tab's content" and (b) "have Windows move that window for me
until mouseup."

## 2. Goals

G1. The torn-off tab arrives in its new window at exactly the
    same width / height it had in the source — to within 1 device
    pixel. (Width preservation; see §5.)
G2. The torn-off window appears at the cursor with no first-paint
    flash. The user perceives "I picked up the window" instantly.
G3. Cross-window merge: dropping on another AgentMux window's tab
    strip moves the tab into that window's strip at the visually-
    indicated insertion point.
G4. Cancel-back-to-source: starting a tear-off and then dropping
    on the source window's tab strip restores the tab to its
    original position; the spawned window vanishes.
G5. **Cross-platform parity.** Win32 ships first, but macOS
    (`[NSWindow performWindowDragWithEvent:]`) and Linux/X11
    (`_NET_WM_MOVERESIZE`) reach perceptual parity *before merge
    to main*. Wayland is the only platform where merge-detection
    is genuinely impossible (no global cursor tracking allowed by
    spec); on Wayland the tear-off still produces a real moving
    window, just without the auto-merge — see §7.
G6. **Pixel-accurate insertion on merge.** When the dragged
    window is dropped on another AgentMux window's strip, the
    insertion index is computed from cursor X against the
    destination's tab geometry — the same way in-window reorder
    works. No "drop at end" shortcut.
G7. **Cancel-back restores exact original position.** Source
    workspace remembers the original tab index for the duration
    of the tear-off; cancel reinserts there, not at the end.

## 3. Non-goals

NG1. Cross-process drag (dragging a tab into Chrome, Slack, etc.)
     — out of scope; the OS doesn't surface a clean way to do
     this for non-OLE participants and Chrome itself doesn't.
NG2. Animated re-flow of the source strip when the tab leaves.
     The remaining tabs simply re-layout without animation; users
     are looking at the cursor, not the strip they just left.

(NG3 from the prior draft — "best-effort Linux" — was *removed*.
Per §0, parity is a goal, not a punt.)

## 4. Architecture (Win32)

### 4.1 Tear threshold

The frontend's existing tab DnD (`tabbar-dnd.ts`) tracks an
in-bar drag using pragmatic-dnd's `monitorForElements`. We extend
that monitor with a tear-threshold check:

```
onDrag({ location }) {
  const r = tabBarScrollRef?.getBoundingClientRect();
  const y = location.current.input.clientY;
  const TEAR_PAST = 24; // px past the bottom edge of the strip
  if (r && y > r.bottom + TEAR_PAST) {
    requestTearOff(draggedTabId);
  } else {
    setInsertionPoint(computeInsertionPoint(location.current.input.clientX));
  }
}
```

`requestTearOff` is a one-shot — once fired, in-bar reorder logic
shuts off for the rest of this drag, and the host takes over.

### 4.2 The tear-off handshake

`requestTearOff(tabId)` invokes a single host command:

```
api.tearOffTab({
    sourceWindowId,
    tabId,
    workspaceId,
    cursorX, cursorY,           // screen coords from getApi().getCursorPoint()
    snapshot: TabSnapshot,      // §5
})
```

Host (Rust, `agentmux-cef`) handles it as follows:

1. **Cancel the in-progress HTML5 drag** in the source webview.
   The host sends a `__tab_drag_canceled` message to the source
   renderer; renderer dispatches `dragend` programmatically and
   pragmatic-dnd's monitor sees the cancellation.
2. **Allocate a destination window from the warm pool.**
   Per §0 the pool is mandatory; a `tearOffTab` invocation
   *must* find a pre-warmed scratch window in the pool and
   *must not* fall back to a cold path under normal conditions.
   The cold path remains in the codebase as a defensive guard
   against pool exhaustion, but firing it logs a `WARN` event +
   increments a `tear_off.pool_exhausted` counter so we can
   detect and fix the underlying race. **First-paint flash
   budget: 0 ms.** The scratch window is shown at
   `(cursorX, cursorY) - tabClickOffset`, sized to the snapshot,
   and the host *immediately* posts a pool-respawn task so the
   next tear-off has a fresh window ready. See §4.5 for pool
   sizing + recycle policy.
3. **Move the tab data** from source to destination workspace.
   Reuse `WorkspaceService.MoveTabToWorkspace(tabId, srcWsId,
   destWsId)` — already exists for cross-window drops.
4. **Hand off cursor capture.** This is the timing-critical bit:
   - Source window: `ReleaseCapture()` (drops OLE drag capture).
   - Destination window: `SetForegroundWindow(destHwnd)`,
     `SetCapture(destHwnd)`.
   - Destination window: `PostMessage(destHwnd, WM_SYSCOMMAND,
     SC_MOVE | HTCAPTION, MAKELPARAM(cursorX, cursorY))`.
5. **Windows takes over.** From this point until mouseup,
   Windows runs its own modal `GetMessage`-based move loop. No
   AgentMux frame is processed; cursor follows the window
   one-to-one.

The handshake (steps 1-4) must complete in a single host-side
call before returning, so the renderer's drag-cancel and the
host's `SC_MOVE` happen back-to-back without a paint frame in
between. **Hard budget: ≤ 8 ms** (one half-frame at 60 Hz).
Chrome empirically sits in 5-8 ms; we match. The handshake is
instrumented end-to-end (timestamp on entry, timestamp on
`PostMessage` return) and emits a `tear_off.handshake_ms`
histogram counter (§10). Any value > 8 ms is a regression.

### 4.3 Tracking the cursor during the move-loop

Because the move-loop is modal, AgentMux's normal renderer
message handlers don't run. To detect "is the cursor over another
AgentMux window's tab strip?", we use a **`WH_MOUSE_LL`
low-level mouse hook**, installed by the host before the
`SC_MOVE` and uninstalled on mouseup.

The hook handler (runs on a background thread):

```
on every WM_MOUSEMOVE:
    let hwnd = WindowFromPoint(cursor)
    let agentmux_window = lookup_agentmux_window(hwnd)
    if agentmux_window != current_target {
        notify_destination_renderer(agentmux_window, cursorX, cursorY)
        current_target = agentmux_window
    }

on WM_LBUTTONUP:
    finalize_tear_off(cursor, current_target)
    uninstall_hook()
```

`notify_destination_renderer` posts an IPC event that the
candidate destination window's tab strip receives — it can then
draw the same insertion-point indicator as a normal in-bar
hover, so the user gets visual feedback during the move.

### 4.4 Drop finalisation

On mouseup, `finalize_tear_off(cursor, target)`:

- **target is another AgentMux window:**
  1. Compute insertion index in target's strip from `cursor.x`.
  2. Move the tab from the dragged window's workspace into
     target's workspace at that index (reuse existing
     `MoveTabToWorkspace`).
  3. Destroy the dragged window (it's now empty).
  4. Show the target window (was likely already visible).
- **target is the source window's strip** (cancel-back path):
  1. Move the tab back to source's workspace at its original
     index (the host kept the original index in the tear-off
     state).
  2. Destroy the dragged window.
- **target is none / empty desktop:**
  1. The dragged window stays where the user released. No-op.
  2. The tab (now the only tab in the dragged window) is the
     window's content.

In all cases, the drag ends and the move-loop hook is removed.

### 4.5 Pre-warmed window pool (mandatory)

Per §0 the pool is the only path that ships under normal
operation. The cold-path fallback exists for defence-in-depth
and is treated as a bug if it fires.

**Pool size: N = 2.** One window is the "next tear-off
destination." The second is the "buffer while the respawn
completes." With N=1 a back-to-back tear-off (user tears off,
then immediately tears off another within ~50 ms) would force a
cold path. N=2 covers that case; the respawn cadence is fast
enough that N>2 is unnecessary RAM.

**Pool window contents:** a fully-initialised, hidden
`agentmux-cef` window with its renderer running, IPC bridge
connected, and the layout system idle (no workspace bound). The
window has been painted at least once at a default size so first
show is instant. Pool windows live forever; on tear-off, the
selected window is *promoted* (workspace bound, layout populated,
shown) and a new pool window is spawned to take its place.

**Lifecycle:**
- App startup: spawn N pool windows in the background after
  primary-window first paint completes, so we don't compete with
  the user's first interaction.
- On `tearOffTab`: pop one window from the pool, promote it,
  immediately enqueue a respawn task. Respawns are serialised
  (max 1 in-flight) so spawn pressure can't spike.
- App shutdown: pool windows close cleanly with the rest of the
  process.

**RAM cost:** ~50-80 MB per pool window for an empty CEF
renderer × 2 = ~100-160 MB resident. Acceptable on desktop.
Logged as `tear_off.pool_resident_mb` so we can monitor.

**Pool-exhausted fallback:** if all N windows are in flight
(unlikely — implies > 2 tear-offs in < the respawn time), fire
the cold path and emit `tear_off.pool_exhausted` (WARN). Do not
silently degrade. Address the underlying race instead of treating
the fallback as normal.

## 5. Width preservation (unchanged from prior draft)

The width snapshot mechanism survives this rewrite — it's
orthogonal to whether we ghost or move-window.

### 5.1 Capture (source side, on tear-off)

```
type TabSnapshot = {
    cssPxWidth: number;     // getBoundingClientRect().width
    cssPxHeight: number;
    devicePixelRatio: number;
    zoomFactor: number;
    color: string | null;
    name: string;
};
```

Captured at tear-threshold-crossing in the source renderer,
travels in the `tearOffTab` payload.

### 5.2 Apply (destination side)

The destination renderer receives the snapshot, writes
`tab:torn-off-width` and `tab:torn-off-at` to the tab's meta,
and applies `style={{ width: \`${snapshot.cssPxWidth}px\` }}` on
the tab DOM element until either:

- 30 seconds elapse, OR
- the user renames / re-drags the tab.

After release, the tab returns to normal `width: auto`.

CSS pixels are DPR-invariant, so cross-monitor tear-offs preserve
the source's *shape* but scale to the destination's chrome.

## 6. Implementation phases

Phases are sequenced for incremental verifiability, **not** for
shipping in increments. Per §0 every phase ships before merge.
Time estimates removed deliberately — they were lazy heuristics
that biased toward "good enough."

### Phase 1 — Tear-threshold detection (frontend) ✅ Shipped in PR #559

- Extends `tabbar-dnd.ts` / `tabbar.tsx` `monitorForElements`
  with the `TEAR_PAST_PX = 24` check.
- `requestTearOff(tabId)` is currently a logging-only stub.
- Verified: dragging a tab past the strip's bottom edge fires
  exactly once per drag.

**Status of subsequent phases:** not started.

### Phase 2 — Host tear-off command + SC_MOVE handshake (Rust)

- New IPC: `tear_off_tab(payload) -> Result<()>`.
- Implement steps 1-4 of the handshake (§4.2) — drag-cancel,
  pool-pop (calls into Phase 6 plumbing), tab transfer,
  capture handoff, `PostMessage(SC_MOVE)`.
- Instrumented end-to-end with `tear_off.handshake_ms` (§10).
- Verify against the §0 budget: handshake completes in ≤ 8 ms
  on the test rig (Win32, DPR 1.0, Release build).

### Phase 3 — Width snapshot (frontend + RPC)

- Extend the tear-off payload with `TabSnapshot`.
- Apply the width on the destination via meta keys + inline
  style. CSS-pixel accurate to ±1 device pixel.
- Tested on cross-DPR / cross-zoom-factor moves.

### Phase 4 — Low-level mouse hook + merge detection (Rust)

- Install `WH_MOUSE_LL` immediately before the `SC_MOVE` post,
  uninstall on `WM_LBUTTONUP` or move-loop abort.
- Track candidate destination via `WindowFromPoint` + AgentMux
  window registry; debounce candidate flips so the destination
  doesn't flicker through stacked windows.
- Push hover events to the candidate destination's renderer so
  it draws the same insertion-point indicator as in-bar reorder.
- Pixel-accurate insertion index (G6) computed from cursor X
  against the destination's tab geometry.
- On mouseup, decide: merge (target exists) or no-op (no target).

### Phase 5 — Cancel-back + finalisation (Rust + frontend)

- Source-window cancel path: drop on origin's strip → tab
  reinserts at original index (G7). Original index is captured
  at tear-time and persists in host-side tear state.
- ESC during move-loop → cancel-back. Treat as user intent to
  abort.
- Source window closed mid-drag → fall back to standalone
  (dragged window stays where dropped).
- Target window destroyed mid-drag → cancel-back to source if
  source still exists, otherwise standalone.

### Phase 6 — Pre-warmed window pool (Rust) — MANDATORY

Per §4.5 + §0. Not optional.

- Pool factory: spawn N=2 hidden, fully-painted scratch windows
  after primary-window first-paint completes.
- Promotion path: `pool.pop()` → bind workspace → resize +
  position → show.
- Respawn: on every promotion, enqueue a single replacement
  spawn (max 1 in-flight serialised).
- Cold-path retains as defence; firing it logs WARN +
  `tear_off.pool_exhausted` (§10).
- Validate: 100 sequential tear-offs at 200 ms cadence — all
  use the warm path, zero cold-path firings.

### Phase 7 — Cross-platform parity (G5)

Each platform reaches the same user-visible behaviour as Win32
before merge. No platform ships as "stub."

- **macOS:** `[NSWindow performWindowDragWithEvent:]` for the
  move-loop equivalent; `CGEventTap` (requires Accessibility
  permission — host requests it once on first tear-off, with a
  clear UX explanation) for global cursor tracking;
  `[NSWindow windowNumberAtPoint:belowWindowWithWindowNumber:]`
  for the hit-test.
- **Linux/X11:** `_NET_WM_MOVERESIZE` for the move; polled
  `XQueryPointer` for tracking; `XQueryTree` walk for
  window-from-point. Tested on GNOME/Mutter, KDE/KWin,
  Sway/wlroots-as-X11.
- **Linux/Wayland:** `xdg_toplevel.move()` for the real move;
  global cursor tracking is **not possible** by Wayland design,
  so merge detection is disabled on Wayland and the torn-off
  window simply remains standalone (G5 carve-out). The user can
  drag the standalone window to merge it manually if their
  compositor supports inter-window drag (it usually doesn't —
  this is a Wayland ecosystem limitation, not ours).
- All three platforms emit the same `tear_off.*` telemetry
  events (§10) so we can track per-platform regression.

## 7. Cross-platform notes

| Capability             | Win32                      | macOS                                          | Linux/X11                  | Linux/Wayland               |
|------------------------|----------------------------|------------------------------------------------|----------------------------|------------------------------|
| Initiate window-move   | `WM_SYSCOMMAND/SC_MOVE`    | `[NSWindow performWindowDragWithEvent:]`       | `_NET_WM_MOVERESIZE`       | `xdg_toplevel::move`        |
| Global cursor tracking | `WH_MOUSE_LL`              | `CGEventTap` (needs Accessibility permission)   | `XQueryPointer` polling    | none — Wayland forbids it   |
| Window-from-point      | `WindowFromPoint`          | `[NSWindow windowNumberAtPoint:belowWindowWithWindowNumber:]` | `XQueryTree` walk          | none reliable               |
| Pre-warm windows       | trivial (`CreateWindowEx`) | trivial (`NSWindow`)                            | trivial (X11)              | trivial (xdg_toplevel)       |

Wayland is the worst case: no global cursor tracking is
permitted, so we lose the merge-detection feature on Wayland.
Acceptable: torn-off tab simply becomes a standalone window;
user can drag again to merge if desired.

## 8. Edge cases

E1. **Drag started on tab, never crosses threshold.** Just an
    in-bar reorder; nothing new happens.
E2. **User holds Esc during move-loop.** ESC triggers
    cancel-back (per §6 Phase 5 + §9): the dragged window is
    destroyed and the tab is reinserted at its original index in
    the source workspace. Windows cancels the move loop on ESC
    and emits `WM_EXITSIZEMOVE` instead of `WM_LBUTTONUP`; the
    hook treats either as the move-loop terminator and routes
    ESC to the cancel-back path.
E3. **Source window closed mid-drag.** State stored host-side
    survives renderer death; cancel-back path becomes "no-op,
    leave the dragged window standalone."
E4. **Two tear-offs in quick succession (impossible per UI but
    defensive).** Pool serializes.
E5. **DPI change during move (cursor crosses monitors with
    different scale).** Windows handles re-scaling
    automatically; renderer's `--zoomfactor` is per-window so
    the dragged window keeps its source's scale until
    mouseup, then the destination workspace's scale applies.
E6. **Tab is the only tab in source workspace.** Tearing it off
    would leave source empty. Two policy choices:
    - *Permit:* source becomes empty, user can close it.
    - *Forbid:* don't allow tear-off; treat as a no-op past
      threshold. Chrome chooses *permit*. We follow Chrome.

## 9. Validation checklist + measurable success criteria

Per phase, with hard thresholds (per §0). Anything outside the
threshold is a P1 fix before merge.

**Phase 1 (shipped in PR #559)**
- [x] Drag within the strip — no tear, normal reorder.
- [x] Drag past the strip's bottom edge — `requestTearOff` fires
      exactly once.
- [x] Re-enter the strip — no further fires.

**Phase 2 (handshake + SC_MOVE) — internal milestone, not a
user-facing release.** This phase verifies the SC_MOVE plumbing
in isolation against the cold path. The first-paint flash and
warm-pool requirements from §0 are *deferred to Phase 6*; Phase
2's exit gate is structural correctness, not perceived
performance. Phase 6's bundle is what ships to users — no
intermediate release after Phase 2.

- [ ] Tear past threshold → new window spawns at cursor with the
      tab's content visible. (Cold-path flash of ~150-300 ms
      is expected here and is *not* an acceptance failure.)
- [ ] `tear_off.handshake_ms` p99 ≤ 8 ms over a 100-tear-off
      sample on the test rig (the 8 ms budget is the §0 hard
      requirement and applies from Phase 2 onward).
- [ ] Window follows cursor 1:1 with no perceptible lag (Windows
      SC_MOVE handles this natively; we verify no input-delay
      regressions).
- [ ] Release → window stays where dropped.

**Phase 3 (width snapshot)**
- [ ] Source tab "Hello" at width 142 px → torn-off tab at 142 px
      ± 1 device pixel in the destination.
- [ ] Cross-monitor (DPR 1.0 → 2.0): width preserved in CSS
      pixels; physical size differs (correct).
- [ ] Cross-zoom-factor (1.0× → 1.25×): width preserved in CSS
      pixels; visual size differs by destination's zoom factor.
- [ ] After 30 s with no user action, the dropped tab relaxes to
      auto-width.

**Phase 4 (merge detection)**
- [ ] Drag torn tab over another AgentMux window's strip —
      insertion indicator appears in the destination within
      one frame (~16 ms) of the cursor crossing the strip.
- [ ] Mouseup over destination strip → tab merges at the
      indicated index (G6); dragged window destroys cleanly with
      no visible flash.
- [ ] Indicator updates as cursor moves along the strip — no
      stuck indicator after a fast horizontal sweep.
- [ ] Drag stays steady over a non-AgentMux window — no
      candidate switch, no merge indicator.

**Phase 5 (cancel-back + finalisation)**
- [ ] Drag past threshold, drag back over source strip,
      mouseup → tab returns to source at *original* index (G7).
- [ ] ESC during move-loop → cancel-back to source.
- [ ] Source window closed mid-drag → torn window survives as
      standalone with no console errors.
- [ ] Target window destroyed mid-drag → cancel-back to source
      (or standalone if source is also gone). No leaked windows.

**Phase 6 (warm pool)**
- [ ] First-paint flash: **0 ms** measured (no visible flash
      with default font sizes on a 60 Hz monitor).
- [ ] 100 sequential tear-offs at 200 ms cadence → all use the
      warm path. `tear_off.pool_exhausted` count = 0.
- [ ] Pool RSS measured at idle ≤ 200 MB (N=2 windows).
- [ ] App startup → pool ready within 2 s of primary-window
      first paint.

**Phase 7 (cross-platform parity)**
- [ ] All Phase 2-6 thresholds met on macOS (Apple Silicon +
      Intel).
- [ ] All Phase 2-6 thresholds met on Linux/X11 (GNOME, KDE).
- [ ] Linux/Wayland: tear-off produces a real moving window;
      merge-detection disabled is documented in user-facing
      copy if surfaced anywhere.

**Cross-cutting (every phase)**
- [ ] No leaked windows after a 30-minute fuzz session
      (random tab grabs, drags, drops).
- [ ] `tsc --noEmit` clean.
- [ ] `cargo check` clean.
- [ ] No new `console.error` / `console.warn` log lines in dev
      mode for any happy path.

## 10. Observability + quality

Per §0, observability is part of the deliverable. Metrics emitted
from the host, consumed by `[fe]`-style log lines and (when
telemetry lands) external counters.

**Counters / histograms** (all under the `tear_off.*` namespace):
- `handshake_ms` — duration from `requestTearOff` IPC entry to
  `PostMessage(SC_MOVE)` return. Histogram buckets at 1/2/4/8/
  16/32 ms. Alert if p99 > 8 ms.
- `pool_resident_mb` — current RAM footprint of the warm pool.
- `pool_exhausted` — incremented every cold-path firing. Alert
  if > 0 in any 24 h window.
- `merge_attempts` / `merge_success` — ratio is the merge
  reliability indicator.
- `cancel_back` — count of cancel-back-to-source events.
- `standalone_drop` — torn windows that ended as standalone
  (legitimate use case, not an error).
- `hook_install_failures` — `WH_MOUSE_LL` / `CGEventTap` /
  `XQueryPointer` setup failures. Should be 0; non-zero blocks
  Phase 4 from working on that platform.

**Structured logs** (target `dnd:tearoff`):
- Tear-off start: `tabId`, `sourceWindow`, `cursorXY`,
  `pool_state` (size, in-flight respawns).
- Handshake stages: drag-cancel sent, pool-pop, tab-transfer,
  capture-handoff, SC_MOVE-posted (with timestamps).
- Merge / cancel / standalone resolution.

**Integration tests** (see `tools/tests/`):
- `tear-off-stress.ps1` (Win32) — programmatic 100-tear-off
  sequence with assertions on handshake_ms p99, pool_exhausted
  count, leaked-window count.
- macOS / Linux equivalents shipped in Phase 7.

**Manual exploration prompts** (for the user):
- Try every drag pattern: slow drift, fast snap, jitter, cross-
  monitor, cross-zoom-factor, drag-and-pause, drag-past-back-and-
  forth, ESC during move, mouse-released-outside-window, etc.
- Try with a browser pane visible behind the strip — torn
  window should paint cleanly over it (airspace already handled
  by the existing pane-overlay primitive).
- Profile RSS in Task Manager during a 30-minute fuzz session
  — should not grow unboundedly.

## 11. Sources

- Existing in-window DnD: `frontend/app/tab/tabbar-dnd.ts`,
  `frontend/app/tab/tabbar.tsx`,
  `frontend/app/tab/droppable-tab.tsx`
- Existing cross-window drag (will be partially replaced):
  `frontend/app/drag/CrossWindowDragMonitor.win32.tsx`
- Window/tab data plumbing:
  `agentmux-srv/src/backend/wcore/tab.rs`,
  `agentmux-srv/src/backend/wcore/window.rs`,
  `frontend/app/store/services.ts` (`MoveTabToWorkspace`)
- Chrome source — TabDragController: where Chrome actually
  implements this. Useful for understanding cancel/merge
  semantics:
  https://source.chromium.org/chromium/chromium/src/+/main:chrome/browser/ui/views/tabs/tab_drag_controller.cc
- Win32 SC_MOVE pattern (canonical reference, MSDN /
  StackOverflow folklore):
  https://learn.microsoft.com/en-us/windows/win32/menurc/wm-syscommand
- `WH_MOUSE_LL`:
  https://learn.microsoft.com/en-us/windows/win32/winmsg/lowlevelmouseproc
- `WindowFromPoint`:
  https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-windowfrompoint
- macOS `performWindowDragWithEvent:`:
  https://developer.apple.com/documentation/appkit/nswindow/1419032-performwindowdragwithevent
- Wayland xdg_toplevel.move limitation context:
  https://wayland.app/protocols/xdg-shell#xdg_toplevel:request:move
- Prior retro on auto-width sub-pixel jitter (motivation for the
  width-snapshot mechanism):
  `docs/retros/RETRO_SUBPIXEL_RENDERING_RESEARCH_2026_04_26.md`
