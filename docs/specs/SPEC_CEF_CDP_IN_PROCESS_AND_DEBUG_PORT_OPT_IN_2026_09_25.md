# SPEC: Browser API over in-process CDP; the CEF debug port is opt-in in release builds

**Date:** 2026-09-25
**Status:** implemented — #3819 (step 1, origins) and this PR (step 2)
**Issue:** #3681 (CEF remote-debugging port always on with `remote-allow-origins=*`, reachable by other local users)
**Decision:** option A of #3681, chosen by Opaz (owner of the browser API, #3445) at the operator's direction; Opaz reviews.
**Related:** `SPEC_BROWSER_DOM_API.md` / `PLAN_BROWSER_DOM_API.md` (the browser API), `SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18.md` (Path 2), `SPEC_AGENT_BROWSER_PANE_DEEP_CONTROL_2026_09_20.md`, `SPEC_INSTANCE_DISCOVERY_FOR_TOOLING_2026_09_17.md` (`authkey.dev` `debug_port`).

---

## 1. Problem

Every AgentMux instance started CEF with a remote-debugging (CDP) TCP server — always, in release builds too — with no authentication and `--remote-allow-origins=*`. Loopback is shared by every account on the machine, so on a multi-user host another local user could list this instance's pages (`GET /json`), attach to the AgentMux UI renderer and run script in it. That renderer holds an authenticated session to the user's `agentmux-srv`, which spawns shells and agents as that user.

`*` added a second, smaller exposure: a page from **any web origin** could open a CDP WebSocket to the port (it would still need a page's GUID). Verified on an unpatched build: `Origin: https://evil.example` → `Runtime.evaluate` succeeds.

## 2. Who used the port

| Consumer | Needs the TCP port? |
|---|---|
| Browser API `/agentmux/browser/*` (browser-pane control, #3445) — in the host process | Used it, over a WebSocket to itself. **No longer** (§3). |
| Agents' `UIQuery`/`UIClick`/`UIScreenshot` — srv → host route → browser API Path 2 | Indirectly, via the browser API. **No longer.** |
| In-app "Inspect Element" | No — CEF native DevTools (`show_dev_tools`). |
| Dev tooling: `scripts/ui-screenshots`, instance discovery (`authkey.dev`), `chrome://inspect` | Yes — and keeps it (§4). |

## 3. Design

### 3.1 Step 1 — origins (#3819)

`--remote-allow-origins` lists only the debug server's own origins (`http://127.0.0.1:<port>,http://localhost:<port>`, what its bundled inspector needs), from the port actually bound; omitted when there is no port. Clients that send no `Origin` (Rust/Node CDP clients) are unaffected. Defense in depth only — it does not stop another local user.

### 3.2 Step 2 — in-process transport (`browser_api/cdp.rs`)

`CdpSession` keeps its `call(method, params)` API but no longer opens a socket. A session addresses a CEF `Browser` by its host-state label.

- `call` registers a one-shot reply slot keyed by a fresh message id, posts a `SendTask` to the CEF UI thread (`SendDevToolsMessage` returns false off it), and awaits the slot with a 10 s timeout (same bound as before), dropping the slot on timeout so a late reply is ignored.
- The UI-thread task finds the browser, registers a `DevToolsMessageObserver` for it once (the `Registration` lives in a UI-thread-local map), and sends `{id, method, params}`.
- The observer's `on_dev_tools_message` completes the matching slot (`{id, result}` → Ok, `{id, error}` → `CDP <method> error: <message>`); events and anyone else's replies are left alone.
- **Lifetime:** the life-span handler's `on_before_close` (every browser — windows and panes) drops that browser's registration and fails its outstanding calls at once; `on_dev_tools_agent_detached` does the same.
- **Session state:** the browser API uses only stateless methods (`Runtime.evaluate`, `Input.*`, `Page.navigate/reload/goBack/goForward/captureScreenshot`) and enables no domain, so one long-lived in-process session per browser behaves like the old fresh socket per request.

The reply-matching core (`PendingCalls`) is plain Rust with no CEF types and is unit-tested.

### 3.3 Step 2 — resolution (`browser_api/resolver.rs`)

A target is now a browser label, not a `/json` target id:

- **Path 1** (browser pane): its own live browser, straight from host state. The URL match and the same-URL stamp probe (`pick_by_stamp`) are gone — nothing to disambiguate when you hold the browser.
- **Path 2** (any other pane): probe each **top-level app window and floater** (`list_top_level_browsers`) for `[data-blockid="<id>"]` (block id `CSS.escape`d in-page). Browser panes' third-party pages and OAuth popups are no longer probed. Not exclusive — many blocks share one window. The last window that held a block is probed first but always verified (a block can move windows).
- `scope_to_block` and every JS helper in `scripts/query.js` (incl. `__amq_allowed_for`) are unchanged.

### 3.4 Step 2 — the port (`cdp_port.rs`, `lib.rs`)

| `AGENTMUX_CDP_PORT` | Release build | Dev build |
|---|---|---|
| unset / empty | **off** | on, prefers 9223 |
| `<1024..65535>` | on, prefers that port | on, prefers that port |
| `1`/`on`/`true`/`yes`/`auto` | on, prefers 9222 | on, prefers 9223 |
| `0`/`off`/`false`/`no` | off | off |
| anything else | on, default port, with a warning | same |

Off means `Settings.remote_debugging_port = 0` (CEF: no server), `debug_port = 0` in state and in `authkey.dev`, no `--remote-allow-origins`, and `"devtools": "off"` in the host popover. When on, the existing bind-probe fallback to an OS-assigned port is unchanged. It is the same variable `scripts/ui-screenshots/capture.mjs` already reads, so one setting serves both sides.

## 4. Behavior changes to know

- Release builds: tooling that CDP-drives a release instance (`capture.mjs`, `chrome://inspect`) needs `AGENTMUX_CDP_PORT` at launch. Dev builds: no change.
- The hosted DevTools frontend linked from `/json`'s `devtoolsFrontendUrl` (appspot) no longer connects (step 1); the bundled inspector at `http://127.0.0.1:<port>/devtools/inspector.html?ws=…` does.

## 5. Verification

Unit: `browser_api::cdp` (reply routing, errors, events ignored, per-browser failure on close, late replies ignored, id wrap), `browser_api::resolver` (many blocks share a window, stale remembered window re-probed, clear no-match error, probe order, selector escaping), `cdp_port` (the table above), `app::remote_allow_origins`. Full `agentmux-cef` suite: 443 passed.

Live, dev build on Windows (2687×1470 display, DPR 1.25):

- **Port on:** browser API over the new transport — two browser panes on the **same URL** resolve to their own pages (marked `pane-A`/`pane-B`, read back correctly; this was the stamp case); `query`/`eval`/`screenshot` ~30–50 ms, screenshots ~190 ms. Path 2: scoped `query` and `screenshot` of the agent pane; `eval` on it still refused; unknown block → `UNKNOWN_BLOCK_ID … (probed N windows)`.
- **Large payloads:** full-window browser-pane screenshot 2682×1307 (473 KB PNG) in 246 ms; an 8 MB `eval` result in 133 ms.
- **Approval subwindow** (a real `memory_adoption_request`): its page has no `[data-blockid]`; a query from an agent pane for `button` returns only that pane's buttons (none of the subwindow's "Adopt"/"Cancel"); block ids `""`, the subwindow's window label, `memory-adoption-approval`, `*` all fail to resolve.
- **Port off** (`AGENTMUX_CDP_PORT=0`, same layout): `authkey.dev` `debug_port: 0`; the host listens only on its IPC port and none of the instance's listening sockets answers `/json/version`; browser API still works — both same-URL panes resolve individually, full-window screenshot, Path-2 query. Host popover reports `devtools: off`.

Not verified: macOS and Linux builds (CI builds them; the code is platform-neutral CEF API); a Path-2 *screenshot* in the port-off run (a browser pane was left magnified, so the agent pane had 0 width — the error came back cleanly through the new channel; Path-2 screenshots were verified in the port-on run, same transport).
