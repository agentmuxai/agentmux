# Performance baseline — 2026-05-09

**Status:** Findings retro (numerical baseline blocked on instrumentation gaps surfaced during the first measurement run)
**Owner:** AgentA
**Spec:** [`docs/specs/SPEC_PERFORMANCE_INSTRUMENTATION_AND_OPTIMIZATION.md`](../specs/SPEC_PERFORMANCE_INSTRUMENTATION_AND_OPTIMIZATION.md) — Phase 1
**Driving observation:** Tab switching and pane resizing both have visibly long delays. AgentMux's brand promise is ultra-snappy responsiveness; this retro establishes the "what's actually happening" before any optimization PR.

## Setup

| Field | Value |
|---|---|
| Build | dev mode — `task dev` against main at v0.33.738 |
| Branch | `agenta/perf-baseline-retro` (read-only retro; no code change in this PR) |
| Instrumentation | Phase 0 (PR #762): perf marks, Long Tasks observer, INP observer, IPC roundtrip clock, dev-mode HUD; Phase 5 (PR #765) diag panel |
| Driving method | Service API (`workspace.SetActiveTab`) — programmatic, not pixel clicks. Per memory `feedback_user_drives_ui_for_baseline.md`, the App API is the right surface for measurement automation. |

The harness path `tools/tests/authfile.ps1::Get-AgentMuxAuthFile` was extended in this branch to search `~/.agentmux/dev/<branch>/data/` and `~/.agentmux/versions/<ver>/data/` in addition to `%APPDATA%\ai.agentmux.cef.*\` so dev-mode runs work without the operator manually copying authkey.dev. (User directive: "fix the key path so we don't need to be slowed down in the future.")

---

## Findings — what the first measurement run surfaced

The numerical capture (P50/P75/P95 for tab switch, pane resize) is **blocked** on three instrumentation gaps that the first measurement run revealed. The retro's value is documenting these gaps so the fix is bounded; the numbers come after.

### Finding 1 — `handleSelect` mark misses programmatic tab switches

**Symptom:** drove 10 tab switches via `Invoke-AgentMuxService -Service workspace -Method SetActiveTab`. Service-side acks all 12 SetActiveTab calls (10 trials + 2 setup). Zero `[fe] [perf] tab-switch …` lines in the host log.

**Root cause:** Phase 0 placed `markStart("tab-switch")` in `frontend/app/tab/tabbar.tsx::handleSelect`, which only fires on a user click on the tab strip. The service API path (sidecar writes to `workspace.activetabid` → frontend reactively subscribes via `getWaveObjectAtom` → `atoms.activeTabId` updates → all per-tab effects re-run) bypasses `handleSelect` entirely.

**Fix (Phase 0.5):** move the mark from imperative (`handleSelect` onClick) to reactive (`createEffect(() => { atoms.activeTabId(); markStart("tab-switch"); /* in microtask, markEnd */ })`). One reactive observer in `workspace.tsx` or similar catches every code path uniformly: human click, programmatic API, keyboard shortcut. ~30 LOC.

This is a **general lesson** for Phase 0 instrumentation: imperative marks at the click handler miss programmatic / reactive / keyboard paths. Reactive marks tied to the underlying state atom catch all callers. Same pattern likely applies to `pane-resize-tick` (currently in `onResizeMove`, misses any non-mouse resize trigger), and to whatever Tier-2 interactions get instrumented next.

### Finding 2 — frontend logs not reaching host log in this run

**Symptom:** zero lines tagged `[fe]` in the host log over a 5+ minute window with the dev instance up. Per `CLAUDE.md`, ALL frontend `console.log` should pipe to host via `fe_log_structured` — but during this run, nothing.

**Possible causes** (not yet root-caused):
1. The CEF window opened but the React app failed to load (Vite dev server connection lag / crash). The agentmux-cef.exe process exists but the page is blank or stuck on bootstrap.
2. `initLogPipe()` ran but the IPC bridge wasn't yet available, and the early console-pipe queue didn't get flushed.
3. The default workspace had 0 tabs (we saw `tabs(start)=1` after first creating, then went up to 2) — possibly the workspace was in a partial-init state where the React app hadn't fully mounted.

**Diagnostic next step:** open the agentmux-cef window manually, observe whether the React UI is rendered. If not, that's the primary issue; if yes, the log-pipe has a bug. Either way, **not a baseline blocker** — once the frontend is reachable, the measurement repeats.

### Finding 3 — service API path is the right driver, but workspaces start empty

**Symptom:** dev mode workspace had 0 tabs at script start. `WorkspaceService.CreateTab` is async-eventual — the workspace object's `tabids` array doesn't update synchronously after CreateTab returns. The script needed a `Start-Sleep -Milliseconds 800` between create and re-fetch to see the new tab.

**Lesson:** the service API is the right driver (per the user's directive — "if you think you need windows-mcp, simply write the app API facility"), but harnesses need to handle the eventual-consistency model. Either poll until the expected state appears, or have the API return the post-update object.

This pattern argues for the App API automation surface in [`SPEC_APP_API_AUTOMATION_SURFACE.md`](../specs/SPEC_APP_API_AUTOMATION_SURFACE.md): a higher-level `tab.create` that waits-and-returns is friendlier than the raw `WorkspaceService.CreateTab` + manual poll.

### Finding 4 — auth file path lookup needed unification

**Symptom:** `Get-AgentMuxAuthFile` only searched `%APPDATA%\ai.agentmux.cef.*\authkey.dev`. Dev mode writes to `~/.agentmux/dev/<branch>/data/authkey.dev`. Every measurement run required a manual `Copy-Item` to the expected location — three times across two sessions of pane-focus / perf work.

**Fix shipped in this branch:** `Get-AgentMuxAuthFile` now searches the dev path AND the per-version portable path. Operators no longer need to copy auth files. (Pushed inline with this retro; `tools/tests/authfile.ps1`, +14 LOC.)

---

## Tier-1 results (pending instrumentation fix)

### Tab switch

> Captured 10 trials via service API. **Zero `[fe] [perf]` measures recorded** because the imperative mark misses the programmatic path (Finding 1). All 12 service calls (10 trials + 2 setup) processed successfully on the sidecar side.

| Metric | Value | Target (spec) | Pass? |
|---|---|---|---|
| P50/P75/P95 latency | _blocked on Finding 1_ | ≤ 100/200/200 ms | _pending_ |
| Long tasks per switch | _blocked on Finding 2_ | 0 ideally | _pending_ |
| Top IPC by P95 | _blocked on Finding 2_ | n/a — informational | n/a |

### Pane resize (splitter drag)

> Not driven this run — the service API doesn't expose `pane.resize` today (proposed in [`SPEC_APP_API_AUTOMATION_SURFACE.md`](../specs/SPEC_APP_API_AUTOMATION_SURFACE.md)). Manual capture would be subject to the same Finding 1/2 blockers.

| Metric | Value | Target | Pass? |
|---|---|---|---|
| All metrics | _blocked on Findings 1+2 + lack of `pane.resize` API_ | n/a | _pending_ |

---

## Hypothesis correlation (unchanged from going-in expectations)

The four hypotheses from the spec stand. Without numerical capture they remain at "predicted" status:

- **H1 (per-frame IPC during pane resize)** — code inspection alone makes this very likely confirmed; `browser-view.tsx::syncPosition` fires on every `ResizeObserver` callback with no debounce, one IPC roundtrip per pane per frame. Awaits measurement.
- **H2 (effect storm on tab switch)** — `activeTabId` change cascades through O(N×M) per-block effects. Awaits measurement; long-task observer (Finding 2 dependency) is the primary signal.
- **H3 (serialized HWND show/hide on tab switch)** — separate IPCs per pane, each blocking on UI thread. Awaits measurement.
- **H4 (hidden Solid reactive leak)** — past incident class; the `untrack` workaround in `recordDispatch` covers the original leak. Awaits long-task counts to rule in/out.

---

## Action items (in priority order)

1. **Phase 0.5 — reactive mark migration** (~30 LOC, 2 hours).
   - Move `tab-switch` mark from `handleSelect` to a `createEffect` on `atoms.activeTabId` so the mark fires regardless of which code path drives the switch (user click, service API, keyboard shortcut).
   - Apply the same pattern to `pane-resize-tick` if any non-mouse resize trigger exists today (none today, but ride the convention).
   - Single PR; small enough to ride alongside the next perf-related code change.

2. **Diagnose frontend log-pipe gap** (Finding 2). Open the dev instance, confirm React UI loaded, tail the log-pipe at construction time. If it's a startup race (initLogPipe runs before IPC is ready), the queue-and-flush pattern likely needs a fix.

3. **Re-run the baseline** after #1 lands. Now expecting non-zero `[fe] [perf]` lines.

4. **Implement Tab primitives from the App API automation spec** (~0.5 day, 5 endpoints). Replaces the raw service API calls with `tab.list` / `tab.create` (await consistency) / `tab.switch`, simplifying every future harness.

5. **Implement Pane primitives** (~1 day) so `pane.resize` is drivable programmatically. Unblocks the pane-resize half of this baseline.

The numbers come after #1 + #2 are resolved. Until then this retro is the reference for what's blocking measurement, not the measurement itself.

---

## Cross-references

- `docs/specs/SPEC_PERFORMANCE_INSTRUMENTATION_AND_OPTIMIZATION.md` — Phase 1 mandate.
- `docs/specs/SPEC_APP_API_AUTOMATION_SURFACE.md` — pushed alongside this retro; covers the harness primitives that make the numbers fast and reliable.
- Memory `feedback_user_drives_ui_for_baseline.md` — the broader principle (use App API, not pixel clicks).
- `tools/tests/authfile.ps1` — auth path lookup unified in this branch.
