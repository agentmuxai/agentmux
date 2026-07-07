# SPEC: Window-close reliability — fix the `backend_window_id` race

**Date:** 2026-07-04
**Status:** Implemented (this session) — see PR (linked once opened)
**Author:** AgentA
**Tracking:** `docs/retro/retro-window-lifecycle-leak-2026-07-04.md` (the incident this fixes), `docs/specs/SPEC_AGENT_SYSTEM_MANAGEMENT_API_2026_07_04.md` §4 (SystemProcessInfo — the longer-term reconciliation follow-up this doesn't replace)

---

## 1. The confirmed root cause

`state.windows` (the srv reducer's canonical window map) is only ever pruned by the `window.CloseWindow` service RPC. In normal operation, the only caller is `backend_close_window` (`agentmux-cef/src/client/helpers.rs:53`), invoked from `on_before_close` (`agentmux-cef/src/client/lifecycle.rs:461`) — and only when a `backend_window_id` lookup against the host's local `shadow_backend_window_ids` map succeeds (`lifecycle.rs:596-601`).

That lookup is a plain, cheap, synchronous mutex read (`state.rs:1395-1397`) against a cache that is **fed asynchronously by the launcher** (host → launcher `register_backend_window` round trip → launcher's authoritative `state.backend_window_ids` → pushed back down into the host's `shadow_backend_window_ids`). The existing code comment at `lifecycle.rs:592-595` asserts this is safe because *"the frontend's original `register_backend_window` ran long before close — shadow has been populated for the entire window lifetime."*

**That assumption is false for a window that is promoted and closed in rapid succession** — exactly the shape of the session that surfaced this bug: 4 pool-promoted windows opened ~2s apart, then all 4 closed within about 7 seconds. If the registration round trip hasn't completed by the time `on_before_close` fires, the shadow lookup misses, `backend_close_window` is never called, and:

- `state.windows` keeps a stale entry forever (confirmed: `Layout(query=windows)` showed 9 entries, only 1 real).
- The cascade `delete_workspace` saga inside the `CloseWindow` handler never runs, so the window's workspace/tabs/blocks also never get cleaned up server-side (confirmed: each of the 8 stale entries carries a full 3-pane workspace).
- The only signal is a `tracing::warn!` ("shells may orphan") — no retry, no escalation, no user-facing indication.

This is a genuine race, not a logic error — the retro's confusion between "pre-promote pool churn" (which never gets a `backend_window_id` and never will) and "a promoted window whose registration just hadn't landed yet" (which will get one, momentarily) was itself evidence of this: both hit the same miss, indistinguishably, at a single point in time.

## 2. The fix

**Give the registration race a chance to resolve before giving up**, on a background thread (never the CEF UI thread `on_before_close` runs on), and **make failures observable** instead of silent.

### 2.1 Bounded retry on the shadow lookup

In `on_before_close`, move the `backend_window_id` resolution off the single synchronous check into a background-thread retry loop: re-check `self.state.backend_window_id(lbl)` every 200ms, up to 5 attempts (~1 second total), before falling back to the existing warn-and-skip. The lookup itself is a cheap mutex read — safe to poll — and this thread already exists for `backend_close_window` today; this just widens its scope to include the resolution step, not only the dispatch.

A 1-second window comfortably covers the registration round trip observed in this session (windows promoted ~2 seconds apart with no visible lag) while staying well clear of ordinary user-perceived latency for a window that's already closing.

### 2.1a Ordering fix: don't unregister ahead of the retry (reagent P1 on PR #1965)

The first version of this fix retained a pre-existing call to `report_backend_window_id_unregistered` inside the immediate-lookup step, unconditionally — including when the lookup missed. That call tells the launcher to drop its own canonical `backend_window_ids[label]` entry and broadcasts `BackendWindowIdUnregistered`, which purges *this host's* shadow map too. Left unconditional, it would race ahead of the very retry meant to catch a delayed registration — potentially unregistering a mapping the moment before (or while) it actually lands, defeating the fix for the exact case it targets.

Fixed by deferring the unregister report until the outcome is actually known: it now fires once, at each of the three terminal points (immediate success, retry success, retry exhausted) — never before the retry has had its chance.

### 2.2 Make `backend_close_window` observably fail

Today it is explicitly fire-and-forget (`helpers.rs:53`, own doc comment: *"we write the request and don't read the response"*). Read the HTTP status line back and log `tracing::error!` (not silently drop) on anything other than a 200, and on any connection/write failure. This doesn't change behavior on the happy path — it just stops swallowing the failure case that made this bug invisible for however long it's been happening.

### 2.3 What this doesn't fix (explicitly out of scope here)

- **The pre-promote pool-churn case still logs a warning it can't act on** (a window-pool-* label that never had, and never will have, a `backend_window_id`). The retry adds a harmless ~1s delay to that already-benign path but doesn't (and can't) resolve it — there's nothing to find. Distinguishing "will never have one" from "doesn't have one yet" cleanly would need the same `pool_destroyed_was_unpromoted` reducer signal `on_pool_window_destroyed` already computes, threaded through to this later check — a reasonable follow-up, not required for correctness here.
- **A full reconciliation pass** (srv periodically cross-checking its `state.windows` against what the host/launcher actually consider alive, and pruning drift) is the durable, structural fix and is exactly what `SystemProcessInfo`-style tooling from `SPEC_AGENT_SYSTEM_MANAGEMENT_API_2026_07_04.md` would enable. This PR closes the specific race that caused today's incident; it does not add a safety net for *other* future ways this mapping could go missing.
- **The main-window `WindowOpened` version-increment-without-close finding** (task #8 in this session's tracking) — root-caused and live-confirmed 2026-07-07, see `docs/retro/retro-window-lifecycle-leak-2026-07-04.md`'s 2026-07-07 update. Not a reload-pairing issue as originally guessed here — it's a host-process crash/restart (under a live launcher) re-labeling a genuinely new browser `"main"`, with no way for the launcher to have paired off the old one first (`handle_goodbye` doesn't touch `state.windows`, and `WindowMirror` has no owning-connection field to let it). Confirmed lower severity than the retro's original "High" filing (ledger/diagnostic drift, not a resource leak or count-inflation bug). Fix needs a schema addition to `agentmux-launcher`; still not implemented, not touched by this fix.

## 3. Resolved / not-resolved open questions from the retro

| Question | Status |
|---|---|
| Was `backend_close_window`'s TCP POST attempted and failing silently for the "clean" closes, or never attempted? | **Moot going forward** — `dlog()` (the only thing that would have shown this) only writes when `AGENTMUX_DEBUG_CLOSE=1` is set, which it wasn't this session, so there's no way to reconstruct it retroactively. §2.2's response-check makes this observable for any future incident without needing that env var. |
| Does closing a window's renderer processes actually survive, contributing to pagefile growth? | **Circumstantially yes, not proven by PID.** Renderer count grew 5→9 in lockstep with the window-record count, with zero extra visible top-level windows — strongly suggestive, not a confirmed PID-to-window mapping (Windows doesn't expose that attribution cheaply). Verifying this is part of this fix's test plan (§4) — if closing a freshly-promoted-and-closed window now correctly returns both the reducer count *and* the renderer count to baseline, that's a strong retroactive confirmation. |
| Main-window version=4 re-registration cause | **Root-caused 2026-07-07** (host crash/restart under a live launcher; see retro update). Fix (a `WindowMirror` owning-connection field + synthetic close on host disconnect) still not implemented — a real schema addition, tracked as its own follow-up, not in scope for this fix. |

## 4. Test plan

- Unit: a fake/delayed shadow-registration test — register `backend_window_id` for a label *after* a short delay, call the close path, assert the retry picks it up instead of giving up immediately (this is the regression test for the actual race).
- Manual/live: repeat the original repro — open several windows via pool-promote in quick succession, close them immediately, and verify via `Layout(query=windows)` that the reducer's window count returns to baseline (not just visually, at the OS window level, which already worked before this fix).
- Same repro, cross-check renderer process count (`Get-CimInstance Win32_Process ... --type=renderer`) before/after to get the first real empirical data point on whether this also explains the renderer-count growth observed today.

## 4b. Round 2 (2026-07-05) — the deeper cause this spec's fix didn't reach

Post-merge empirical verification invalidated §1's implicit assumption that `on_before_close` fires at all for these closes. It does not: CEF Views `window.close()` on this build destroys the Window but leaves the browser hidden/recycled (`lib.rs` ~1050 documents this for the quit path; Discussion #1680), so the entire close-cascade — including this spec's retry — never executes for mid-session secondary-window closes, and each close leaks a live renderer on top of the srv-side state.

Round-2 fix (same PR series): `CloseWindowTask` now follows `window.close()` with `close_browser(1)` on the closed label's browser (non-`main` only), forcing real browser destruction so `on_before_close` fires and the §2 cleanup chain — which is correct, and was simply dead code for this path — runs. §2's retry remains valuable for its original race; §2.2's observability made the "srv never heard about any close" diagnosis possible.

Verification protocol for round 2 (per §4, executed against a fresh isolated build with `AGENTMUX_DEBUG_CLOSE=1`):
- `on_before_close fired` entries present in the close-debug trace for each closed window
- "Unregistered browser" in the host log per close
- `backend_close_window` connect + HTTP 200 response lines per close
- srv `GET /api/v1/windows` returns to baseline count
- instance-scoped renderer process count returns to baseline

## 5. Follow-ups tracked separately (not this PR)

- Task #7 (this fix)
- Task #8 — main-window reload/version-increment investigation
- Task #9 — window-level lifecycle test coverage (broader than the one regression test in §4)
- Task #10 — cross-reference against the pagefile/OOM spec
- Reconciliation pass / `SystemProcessInfo` (SPEC_AGENT_SYSTEM_MANAGEMENT_API_2026_07_04.md) — still undecided whether/when to implement
