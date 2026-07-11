# SPEC: Pool adoption for foreign labels + srv window-row label crumb + non-Windows close verification

**Date:** 2026-07-11
**Status:** Ready for implementation
**Tracking:** session task #15 (the residuals deliberately left open by the
window-pool coverage work, ~PR #1969 era, and referenced by
`CloseWindowTask`'s "Known residual" comment in `agentmux-cef/src/ui_tasks/window.rs`)
**Related:** #2087/#2088/#2089 (the `Client.windowids` leak class — closed),
`SPEC_PARK_AND_BLANK_CLOSE_2026_07_09.md`, `retro-window-lifecycle-leak-2026-07-04.md`

Three independent residuals, ordered by value. They share a theme — the window
close/recycle machinery keys on the `window-pool-` label prefix and on
host-side registration state, and both assumptions have known gaps — but they
are separately shippable PRs.

---

## Residual 1 — pool adoption for foreign `window-{uuid}` labels

### Problem

`CloseWindowTask` (Windows) demotes a closing window back into the warm pool
only when its label starts with `window-pool-` — the pool handshake
(`pool_window_ready` → queue entry → promote) keys on that prefix. Cold-path
and drag-tear-off windows get `window-{uuid}` labels, so their close falls to
the park-and-blank path: srv state IS cleaned (post-#2087), but the parked
renderer (~100MB commit) is never reclaimed and never reused.

In the default flow this is rare — `open_new_window` serves from the pool
whenever it's non-empty — but every pool-exhausted open (rapid multi-window
creation, pool refill latency) mints a foreign label whose eventual close
permanently strands a renderer. Over a long session these accumulate exactly
like the pre-#2000 renderer leaks did, just at a lower rate.

### Design — adopt at demote time

At the `CloseWindowTask` demote gate, when the label is `window-*` but not
`window-pool-*` and the pool is below its demote cap:

1. Run the same eligibility steps as `demote_promoted_pool_window` (strict
   HWND resolution, reducer `DemotePoolWindow` — which must learn to accept
   an adoption, flipping `is_pool: true` for a label it has never seen as
   pool-side).
2. **Relabel on reload:** the browser reloads to the pool boot URL with a
   fresh `window-pool-{uuid}` label in the query string. The renderer's
   pool-wait bootstrap sends `pool_window_ready` with the NEW label; from the
   handshake's perspective this is indistinguishable from a fresh spawn.
   The old foreign label is scrubbed from every host map (`window_hwnds`,
   reducer browsers registry re-keyed via UnregisterBrowser + fresh
   RegisterBrowser under the new label — NOT mutation in place, so every
   existing invariant about label lifetime holds).
3. Launcher ledger: `report_window_closed(old_label)` (already happens) +
   `report_pool_window_added(new_label)` — the count mirrors stay paired.

Rejected alternative — teaching the whole pool machinery to accept arbitrary
`window-*` labels — touches every prefix check in window_pool.rs, the reducer,
and the frontend pool gate (`isPoolMode` reads `pool=1`, not the label), for
no benefit over relabeling at the single adoption point.

### Verification

`AGENTMUX_DEBUG_CLOSE=1` + the E2E harness: exhaust the pool (open windows
until a `window-{uuid}` label appears), close it, assert (a) renderer count
returns to baseline (wmic, `--type=renderer` filtered), (b) the pool gained a
warm entry, (c) `Client.windowids` unchanged (regression guard on #2087).

---

## Residual 2 — srv window-row label crumb

### Problem

srv `Window` rows carry no record of which host window label created them.
Every cleanup path resolves label → `window_id` exclusively through the
host-side registration chain (`register_backend_window` →
launcher → `shadow_backend_window_ids`). When that chain hasn't completed —
the #2088 race, closed but only for the initHostNewWindow path — or when the
host process is simply gone (crash forensics), nothing can map a label to its
row. `demote_srv_cleanup`'s fallback today is "log `srv state may orphan` and
give up."

### Design

Write the label as a meta key on the Window row at creation time:

- Frontend `initHostNewWindow` already knows both (`windowLabel` from the URL,
  `newWindow.oid` from CreateWindow) — pass the label as a new optional arg to
  `window.CreateWindow`, persisted into `Window.meta["host:label"]`.
- srv: `CreateWindow` handler threads it through the reducer command; the
  persist subscriber writes it with the row. No new table, no migration —
  `meta` is already a `MetaMapType`.
- Consumers (all optional, additive):
  - `demote_srv_cleanup`'s no-registration fallback can ask srv
    `window.FindByLabel` (new lightweight read RPC filtering on the meta key)
    before giving up.
  - Crash forensics / `muxlog`-era debugging: rows become attributable.
- The crumb is a **hint, not an identity**: labels are reused across host
  restarts (`main`) and adoption (Residual 1) relabels windows, so consumers
  must treat a miss or a stale value as "fall back to current behavior,"
  never delete on crumb evidence alone.

### Verification

Unit test on the srv handler (crumb persisted + FindByLabel resolves);
E2E: close a window within the registration gap (the #2088 repro shape) and
assert the fallback now closes the row via the crumb.

---

## Residual 3 — non-Windows close-path verification

### Problem

Every close-path finding since 2026-07-04 (CEF-148 parking, demote, imperative
srv cleanup, #2087–#2089) was established and verified on Windows only. On
macOS/Linux the code takes different branches (`window.close()` Views path,
`on_before_close` believed to fire properly — "no parked-browser evidence
there yet", per `demote_promoted_pool_window`'s doc comment). "Believed" is
not "verified": if parking also happens there, those platforms still leak srv
rows on every secondary-window close.

### Scope (verification only — no code unless it fails)

On macOS and Linux (`task dev`, launcher-in-loop):

1. Open + close a secondary window via chrome ✕, IPC `closeWindowByLabel`,
   and the OS close (Cmd+W / window-manager close button).
2. Assert `Client.windowids` returns to baseline each time (the
   `window-close-baseline` E2E suite is Windows-gated today — lifting the
   `IS_WINDOWS` gate for these two suites IS the deliverable if the paths
   pass; a platform-specific fix spec is the deliverable if they don't).
3. Confirm `on_before_close` actually fires there (close-debug trace).

---

## Suggested PR slicing

1. PR A — Residual 2 (crumb): smallest, pure additive, unblocks better
   fallbacks everywhere.
2. PR B — Residual 1 (adoption): Windows-only, riskiest, needs live verify.
3. PR C — Residual 3: verification pass; either lifts the E2E platform gate
   or produces a new fix spec.
