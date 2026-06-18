# SPEC: External-link routing + single robust "Restore" recovery

**Date:** 2026-06-17
**Status:** Implemented (this branch)
**Area:** `agentmux-cef` (host: popup routing), `frontend` (startup recovery UI)
**Related:** `SPEC_BRIDGE_INIT_RECOVERY_2026_06_15.md`, `SPEC_BROWSER_PANE_DEFAULT_URL_AND_POPUP_2026_04_21.md`

---

## 1. Summary

Two coupled defects, found from a real lock-out on 2026-06-17:

1. **Help links hijack the whole app window.** Clicking "Report Bugs & Issues"
   (and every other external link in the Help pane) navigated the *main app
   window* to GitHub instead of opening the system browser. The user lands on a
   full-screen GitHub login with no way back.

2. **"Can't reconnect to AgentMux" cannot actually recover.** After the window
   left the app, its host bridge (`window.api`) never rebuilt. The recovery
   card's two buttons ("Reload" and "Reopen window") both failed for the same
   reason, so the window was permanently stranded. Manual rescue required
   driving the CEF DevTools protocol from outside the app.

This spec fixes (1) so the lock-out cannot start, and replaces (2) with a single
**Restore** button engineered to work for every case an in-app button *can*
fix, with a documented escalation for the rest.

---

## 2. Incident walkthrough (evidence)

### 2.1 The navigation hijack

`frontend/app/element/quicktips.tsx` renders external links as plain
`<a target="_blank" href="https://github.com/...">`. In CEF, `target="_blank"`
fires `LifeSpanHandler::on_before_popup`. The host's handler
(`agentmux-cef/src/client/mod.rs`) cancels the popup and **navigates the current
frame** to the URL:

```rust
// (before) on_before_popup
let mut task = crate::ui_tasks::DeferredLoadUrlTask::new(browser_clone, url.clone());
cef::post_task(cef::ThreadId::UI, Some(&mut task));   // loads github.com IN the app window
```

QuickTips is part of the main app DOM, so "the current frame" is the entire
AgentMux UI. It was replaced by `https://github.com/login?return_to=...`.

### 2.2 Why the window came back dead

CEF host log for the recovered window (re-navigated back to the app):

```
01:07:10  Injected IPC port 54469 into page: http://127.0.0.1:54469/
01:07:15  [initApp] window.api still undefined after 5s - host API bridge failed to initialize
          Bootstrap failed
```

The host *did* inject IPC creds, yet `window.api` never built. Root cause is a
race in the reload path:

- On first load, the host opens the window with creds in the URL
  (`?ipc_port=..&ipc_token=..&windowLabel=..`). `cef-init.ts setupCefApi()`
  reads them **synchronously** (`frontend/cef-init.ts:50-58`), then strips them
  from the URL for security (`:66-71`).
- On any later load (reload, or the back-navigation here) the URL no longer
  carries creds. `setupCefApi()` then depends on the host's `on_load_end`
  re-injection landing inside its `await waitForIpcCreds(2500)` window
  (`frontend/cef-init.ts:73-77`).
- The injection is unconditional but asynchronous
  (`agentmux-cef/src/client/mod.rs` `on_load_end`, executes JS after load
  commit). When it lands *after* the 2.5s budget, `setupCefApi` gives up,
  `window.api` is never set, and `app-init.ts:435` throws after its own 5s
  poll.

### 2.3 Why both recovery buttons failed

`frontend/app/init/error-display.ts` (before):

- "Reload" -> `location.reload()` -> reloads the **creds-stripped** URL -> hits
  the exact same `waitForIpcCreds` race -> fails again.
- "Reopen window" -> `location.assign(location.pathname + location.search)` ->
  same creds-stripped URL -> same race -> fails again.

Both buttons relied on the racy on_load_end path, so neither could ever break
out of it. The auto-recover loop (`tryAutoRecover`, 3 attempts) used
`location.reload()` too, so its three tries failed identically before the card
even appeared.

### 2.4 Secondary trap: workspace is single-window

Recovery via "open a fresh window" hit a second wall: a workspace can be live in
only one window. The dead window still held the user's workspace, so the in-app
workspace switcher just **focused the dead window**. The dead window had to be
closed first to free the workspace. This is why "open a new window" alone is not
a sufficient Restore.

---

## 3. Goals / non-goals

**Goals**
- External links from the app UI open in the system browser, never in-app.
- One button, "Restore", that recovers the window deterministically for the
  common (host-alive) case, lands the user back on their **same workspace**, and
  degrades sanely when the host is genuinely down.
- No reliance on `window.api` inside the recovery path (it is the thing that
  failed).
- No new host round-trips for the common case (keep it fast and offline-safe).

**Non-goals**
- Changing the workspace/window data model.
- Fixing the underlying `waitForIpcCreds` timing in `setupCefApi` itself
  (the credentialed-URL approach sidesteps it; tightening that budget is a
  separate, optional follow-up).
- Recovering when the host process is actually dead. No in-page button can
  rebuild a dead host; the card says so and points at "restart AgentMux".

---

## 4. Design

### 4.1 Fix 1 - external links to the system browser

There is already a vetted primitive: the `open_external` IPC command opens a URL
in the OS default browser with scheme validation
(`agentmux-cef/src/commands/platform.rs`, Windows `rundll32 url.dll,FileProtocolHandler`,
macOS `open`, Linux `xdg-open`).

Changes:

1. **`platform.rs`**: split the body of `open_external` into a reusable
   `open_url_in_default_browser(url: &str) -> Result<(), String>` (scheme
   allowlist preserved), and add a classifier:

   ```rust
   /// http(s) URL whose host is NOT this app's loopback origin -> external.
   pub fn is_external_http_url(url: &str) -> bool { ... }  // 127.0.0.1/localhost/0.0.0.0 => internal
   ```

   Unit tests cover github/docs/discord (external), `127.0.0.1:<port>` and
   `localhost:<vite>` (internal), and non-http schemes (not routed).

2. **`client/mod.rs` `on_before_popup`**: before the existing in-frame
   navigation, branch on origin:

   ```rust
   if !self.is_browser_pane && crate::commands::platform::is_external_http_url(&url) {
       let _ = crate::commands::platform::open_url_in_default_browser(&url);
       return true; // cancel popup; do NOT navigate the app frame
   }
   // else: browser pane or internal URL -> existing DeferredLoadUrlTask nav
   ```

   - **Browser panes are exempt:** following a link there *is* web browsing, so
     they keep in-pane navigation.
   - **Deadlock safety:** `open_url_in_default_browser` only spawns a child
     process; it never re-enters CEF or `self.inner`, so calling it inline under
     the handler lock is safe (unlike an inline `load_url`, which is why that one
     is deferred via `post_task`).

   This fixes the Help pane links plus every other `target="_blank"` /
   `window.open` to an external site, app-wide. No per-link frontend change
   needed.

### 4.2 Fix 2 - single "Restore" that works

The key realization from the logs: **the host re-injects
`window.__AGENTMUX_IPC_PORT__` / `__AGENTMUX_IPC_TOKEN__` on every main-frame
load, unconditionally** (`on_load_end`). By the time the user can click Restore,
those globals are present even though `window.api` (built later, and racily)
is not. So Restore can use the low-level creds directly.

**Primary action - heal in place (no host round-trip):**

```
buildCredentialedUrl():
  read window.__AGENTMUX_IPC_PORT__ / __TOKEN__   (re-injected by host)
  read windowLabel from current URL               (preserves workspace binding)
  return  <origin>/?ipc_port=..&ipc_token=..&windowLabel=..   (or null if no creds)
```

`location.assign(credentialedUrl)` reloads the app with creds **in the URL**, so
`setupCefApi` reads them synchronously (no `waitForIpcCreds` race) and bootstrap
is deterministic. Because `windowLabel` is preserved, the window rebinds to the
**same workspace** (`registerBackendWindow(windowLabel, windowId)` re-links it).
This is the same shape as the host's own new-window URL, so the brief token-in-URL
exposure is identical to existing behavior and cef-init strips it on load.

**Escalation - host-spawned fresh window:**

If the in-place heal was already tried (a `sessionStorage` latch) or no creds are
present, Restore asks the live host for a brand-new, freshly-bridged window using
the creds directly (NOT `window.api`):

```
POST http://127.0.0.1:<port>/ipc   Authorization: Bearer <token>
     { "cmd": "open_new_window", "args": {} }
then window.close()   // free this window's workspace for reselection
```

`open_new_window` is the same command the launcher forwards for a second
`agentmux.exe` launch, and `window.close()` on a host window is already used by
the install-broken page's Quit button. Closing the dead window frees its
workspace, so the (now working) switcher can reopen it. This is the manual
2026-06-17 rescue, automated.

**Last resort:** if the host is unreachable for the POST, fall back to
`location.reload()` and let the card reappear with "restart AgentMux".

**Auto-recover upgraded too:** `tryAutoRecover` now uses
`buildCredentialedUrl()` for its bounded retries instead of `location.reload()`.
In the 2026-06-17 incident this means the *first automatic* attempt heals the
window and the card never appears.

### 4.3 Button + copy

- Two buttons collapse to one: **"Restore"** (primary blue). On click it
  disables, shows "Restoring...", and runs `doRestore()`.
- Card copy (budget-exhausted state) points at Restore and explains the
  host-down fallback, instead of the old "close this window and reopen it"
  instruction that did not work.

---

## 5. Recovery decision flow

```
bootstrap fails (window.api not ready)
  -> tryAutoRecover (<=3): location.assign(credentialedUrl)   [deterministic]
       success -> clearStartupReloadCount() (resets budget + restore latch)
       still failing after 3 -> show card
  -> card "Restore" click:
       1st press, creds present -> location.assign(credentialedUrl)  [heal in place, same workspace]
       2nd press (or no creds)  -> POST open_new_window + window.close()  [escalate]
       host unreachable         -> location.reload()  [card returns: "restart AgentMux"]
```

---

## 6. Why this is robust

- **Removes the race instead of retrying into it.** Every recovery path now
  carries creds in the URL, the same deterministic path used on first load.
- **No `window.api` dependency** anywhere in recovery - only the injected creds
  and `fetch`, which survive the bridge failure.
- **Same-workspace by construction** via preserved `windowLabel`; no fragile
  "open new window then switch" dance for the common case.
- **Bounded, no storms.** The reload budget (`MAX_RELOADS = 3`,
  `sessionStorage`-guarded) is unchanged; the restore latch makes the second
  press escalate rather than repeat.
- **Honest failure.** If the host is truly dead, the page says restart - the one
  thing it cannot do for itself.

---

## 7. Files changed

| File | Change |
|------|--------|
| `agentmux-cef/src/commands/platform.rs` | Extract `open_url_in_default_browser`; add `is_external_http_url` + unit tests |
| `agentmux-cef/src/client/mod.rs` | `on_before_popup` routes external URLs to the system browser; doc updated |
| `frontend/app/init/error-display.ts` | Single "Restore"; credentialed re-navigate + escalation; auto-recover upgraded; copy updated |
| `frontend/app/init/error-display.test.ts` | Reconcile assertions with new copy/markup |

---

## 8. Test plan

**Automated**
- `is_external_http_url` unit tests (in `platform.rs`).
- `error-display.test.ts`: card renders, single Restore button present,
  `buildCredentialedUrl` returns a creds+label URL when globals are set and
  `null` when absent.

**Manual**
1. Help pane -> click each external link (GitHub, docs, Discord). Each opens in
   the **system browser**; the app window is untouched.
2. Simulate a bridge failure (e.g. delay/withhold first-load creds so bootstrap
   throws). Confirm the auto-recover credentialed re-navigate reconnects without
   the card.
3. Force the card (exhaust the budget). Click **Restore**:
   - With host alive: window reloads onto the **same workspace** with all panes.
   - Click again to exercise escalation: a fresh window opens and the dead one
     closes.
4. Browser pane: `target="_blank"` inside a browsed page still navigates **in
   the pane** (not the system browser) - exemption holds.

---

## 9. Follow-ups (out of scope)

- Tighten `setupCefApi`'s `waitForIpcCreds` budget / make on_load_end injection
  ordering deterministic, so even a creds-stripped reload self-heals.
- Optional: have the escalation pass the dead window's workspace id to
  `open_new_window` so the fresh window lands on it directly (removes the manual
  reselect in the rare escalation case). Requires threading a `workspaceId`
  through `open_window_with_kind` and the new-window URL.
