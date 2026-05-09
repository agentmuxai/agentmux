# Host API + Win32 focus probes

**Status:** Proposed
**Owner:** AgentA
**Date:** 2026-05-09
**Predecessor:** [`SPEC_TEST_API_ACCESS.md`](SPEC_TEST_API_ACCESS.md), [`SPEC_PANE_FOCUS_STRESS_TEST.md`](SPEC_PANE_FOCUS_STRESS_TEST.md)
**Driving incident:** PR #760 — browser-pane address-bar/page-DOM focus routing fix; the harness false-negatived 11/24 steps because it had no way to programmatically click main-React DOM elements or read which Win32 HWND held OS focus.

## Why

The existing browser API (`/agentmux/browser/*`, see `agentmux-cef/src/browser_api/`) is **complete for embedded pane DOMs** — eval, query, focus_info, click_element, focus_element, navigate, etc., all keyed by `block_id`. It targets the *pane CEF browser*.

Two probes are missing that turned a 30-minute fix into a 2-hour session:

1. **No mirror API for the main React webview** (the host chrome — title bar, address bars, terminal `<input>` elements, modals, agent picker). The harness drives the address bar by computing pixel coordinates from window geometry — fragile across window sizes, DPI, and any layout edit. In our run, auto-computed coords missed the address-bar `<input>` entirely (zero `input-focus` events fired across 24 steps).
2. **No probe for Win32 OS focus** — the second axis of the focus-routing bug. A click can move *DOM* focus (CEF processes the event, React `onFocus` fires) while leaving *OS keyboard focus* on a sibling HWND (e.g. the pane HWND retains it because the WndProc didn't see the click). Today we infer this disjunction post-hoc from `[pane-wndproc] key msg=` log lines. With a probe we could assert it directly.

This spec adds both, plus the harness path fixes that bit us during PR #760 verification.

---

## API additions

### Namespace `/agentmux/host/*` — main React webview probes

Mirrors `/agentmux/browser/*` but targets the main host webview instead of a pane CEF. No `block_id` needed; there's exactly one main webview per window.

| Endpoint | Body | Returns |
|---|---|---|
| `POST /agentmux/host/eval` | `{ window_label?: string, expression: string, return_by_value?: bool }` | Same shape as `/browser/eval` — `{ ok, result \| error }` |
| `POST /agentmux/host/focus_info` | `{ window_label?: string }` | `{ ok, active_element: { tag, id?, classes[], value?, selectionStart?, selectionEnd? } }` |
| `POST /agentmux/host/click_element` | `{ window_label?: string, selector: string }` | `{ ok }` — synthesizes a real `Input.dispatchMouseEvent` (same Win32 routing as a human click; identical to `/browser/click_element` semantics) |
| `POST /agentmux/host/query` | `{ window_label?: string, selector: string, limit?: number }` | `{ ok, elements: [{ index, tag, rect: {x,y,w,h}, ... }] }` |

**`window_label` defaults to `"main"`** — the IPC misroute that bit us in PR #484 (see `ipc.rs:424-428`). Multi-window setups MUST pass `window_label` to target the right webview; the harness reads it from `window.location.search` already.

**Implementation:** open a CDP connection to the main browser via the existing `cef_app_state.main_browser_per_label` map (already keyed by window_label for the per-window-focus IPC). Same auth/token mechanism as `/browser/*`. New module `agentmux-cef/src/host_api/` with `mod.rs` (route registration), `routes.rs` (handlers), `scripts/query.js` (the centroid/focus_info JS helpers — can lift from `browser_api/scripts/`).

### Namespace `/agentmux/win32/*` — Win32-level probes

| Endpoint | Body | Returns |
|---|---|---|
| `POST /agentmux/win32/focus_state` | `{}` | `{ ok, foreground_hwnd, foreground_pid, focused_hwnd_thread, pane_map: [{ block_id, hwnd, window_label }] }` |
| `POST /agentmux/win32/list_windows` | `{}` | `{ ok, windows: [{ window_label, main_hwnd, pane_hwnds: [{block_id, hwnd}], geometry: {x,y,w,h} }] }` |
| `POST /agentmux/win32/send_keys` | `{ window_label?: string, target: "main" \| { block_id }, text: string }` | `{ ok, routed_to_hwnd, key_msgs_received: [{hwnd, msg, wparam}] }` — synthesizes input via `SendInput` and reports which HWND processed each WM_KEYDOWN/CHAR (correlated against `pane_map`). |

**Why these:**

- `focus_state` is the disjunction probe. `foreground_hwnd` is the top-level window with foreground; `focused_hwnd_thread` is the focused HWND in that window's thread (the keyboard input target). When they don't match expectations, you have a focus bug. `pane_map` lets the harness assert "OS focus is on pane X's HWND" or "OS focus is on the main webview HWND, not any pane".
- `list_windows` closes the multi-window pool-drift diagnosis loop — the bug AgentA hit in v0.33.726 had `drift_detected kind:pool host_count:1 mirror_count:2`; an API to compare host-side pane HWND mappings against launcher-mirror counts would surface this immediately rather than via launcher event log post-hoc.
- `send_keys` is the cleanest test primitive. Today the harness uses `SendKeys.SendWait` (.NET, sends keys to whichever HWND has thread focus); it has no idea which HWND actually received them. `routed_to_hwnd` + the per-keystroke wndproc trace gives us a closed-loop assertion: "I asked to send 'r1a' to pane X — it landed on HWND X with three WM_KEYDOWN events."

**Implementation notes:**

- `focus_state` uses `GetForegroundWindow`, `GetGUIThreadInfo` (for the focused HWND in the foreground thread). `pane_map` reads from `agentmux-cef`'s existing pane-HWND registry (`browser_pane::hwnd::PANE_HWND_REGISTRY`).
- `send_keys` uses `SendInput` with `KEYEVENTF_UNICODE` for arbitrary text, hooks the wndproc trace for the duration of the call to capture which HWND received each event. Hook can reuse the existing `[pane-wndproc] key msg=` infrastructure plus a similar trace on the main webview HWND.
- All three are gated by the same auth-token mechanism as `/browser/*`.

---

## Harness path fixes (separate from API)

### `tools/tests/authfile.ps1`

`Get-AgentMuxAuthFile` searches `%APPDATA%\ai.agentmux.cef.*\authkey.dev`. Add the dev-mode path:

```ps
$candidates += Get-ChildItem -Path "$env:USERPROFILE\.agentmux\dev\*\data\authkey.dev" -ErrorAction SilentlyContinue
```

`Get-AgentMuxHostLogPath` already takes `$Auth` and reads `Auth.data_dir`; just verify it correctly resolves dev-mode logs at `<data_dir>/logs/agentmux-host-<instance>.log.*`. Currently it only checks `~/.agentmux/logs/`.

### `tools/tests/pane-focus-stress.ps1`

`Find-AgentMuxMain` searches for `agentmux-cef` process name. Portable rebrands to `agentmux-<version>.exe`. Match either by including a regex search:

```ps
Get-Process | Where-Object { $_.ProcessName -match '^agentmux(-cef|-\d+\.\d+\.\d+)$' }
```

The harness's auto-computed click coordinates (`$winY + 87` for address-bar Y, `$winY + 413` for search-bar Y) are calibrated against a specific layout. Once `/agentmux/host/click_element` ships, switch the harness to selector-based clicks (`input.browser-address-bar`, the terminal block's `.xterm-screen`, etc.) — coordinates become a fallback for things still not selector-addressable.

---

## Threat model

Identical to `SPEC_TEST_API_ACCESS.md` §3. The new endpoints expose the same JS-eval-in-renderer surface area we already grant `/browser/*`; same auth-token gate, same dev-only auth file, same threat class (same-user local process, which we don't defend against). `/win32/send_keys` is the only new escalation — it can synthesize input without user consent. Same-user-local trust class still covers it; if we ever broaden the trust boundary, `send_keys` would need an extra opt-in flag.

---

## Out of scope

- A Selenium/WebDriver-style high-level scripting layer. The point of these probes is **lowest-level deterministic primitives** that test harnesses build on; bundling logic into the host invites the kind of test-framework lock-in we've avoided so far.
- Cross-window message passing — the existing window_label parameter is enough for the test cases we have today.
- Recording/replay. Out of scope here; if needed later, it sits on top of `host/eval` + `win32/send_keys`.

---

## Effort estimate

| Phase | LOC | Days |
|---|---|---|
| `/agentmux/host/*` (4 endpoints) | ~250 (mirror existing browser_api/) | 0.5 |
| `/agentmux/win32/*` (3 endpoints) | ~200 (Win32 calls + registry reads) | 0.5 |
| Harness path fixes | ~30 | 0.25 |
| Tests + smoke | — | 0.25 |
| **Total** | ~480 | **~1.5 days** |

Single PR; rides with the implementation per `feedback_no_doc_only_prs.md` (no doc-only PRs).

---

## Cross-references

- `SPEC_TEST_API_ACCESS.md` — auth-key/auth-file mechanism (predecessor)
- `SPEC_PANE_FOCUS_STRESS_TEST.md` — harness this unblocks
- `agentmux-cef/src/browser_api/` — pattern to mirror
- PR #760 — driving incident
- GitHub Discussion #707 — long-term reducer-stack thread; this work is adjacent (better diagnostics for the focus axis the catalog flagged as "DOM is source of truth, don't reduce it")
