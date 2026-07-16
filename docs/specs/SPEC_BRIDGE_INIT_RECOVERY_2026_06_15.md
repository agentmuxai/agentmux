# SPEC: Host-Bridge Init Failure — Self-Heal + Recovery UI

**Date:** 2026-06-15
**Status:** Implemented (self-heal loop, recovery UI, Ctrl+R/F5 keybinding all shipped)
**Correction (2026-07-16):** §3.5's premise that *"the document is a real `http://…` URL, so `location.reload()` does work and preserves the `__AGENTMUX_IPC_PORT__`/token query params"* was **proven false** by the #52 investigation (PR #2181): `setupCefApi` strips `ipc_port`/`ipc_token` from the URL after first read (the 2026-06-12 token-leak fix), so a reload arrives cred-less and — for `is_browser_pane`-flagged floating-pane/pool windows, which the host's `on_load_end` skipped — could never reacquire them. The self-heal reload loop this spec designed was re-entering an identical, deterministic failure every ~5s. Fixed in #2181 by (a) origin-gating the host's cred re-injection instead of gating on the pane flag, (b) making `isCef()` sticky via sessionStorage across the strip, and (c) injecting the authoritative `state.ipc_port` (pool handlers carry `self.ipc_port = 0`). With those in place, the reload-based self-heal below works as designed for all windows.
**Scope:** `frontend/app/init/error-display.ts` + `frontend/app-init.ts` startup-error path

---

## 1. Problem

When the frontend can't establish `window.api` (the CEF host bridge), the app
dies on a bare, dead error screen:

```
AgentMux failed to start
Error: [initApp] window.api still undefined after 5s — host API bridge failed to initialize
Press F12 for console details. Try closing and reopening the app.
```

There is **no recovery action** (no reload button, no auto-retry) — the only
way out is F12 + manual `location.reload()` or killing the window.

### Observed incident (2026-06-15)

A running **dev** instance logged this error **385 times in a storm** at 03:35,
~31 min after a clean startup where the bridge installed fine (`window.api
available: true`, backend on `127.0.0.1:51078`).

**Root cause:** `git checkout` / `git pull` / `git rebase` were run in the same
clone the dev instance's **Vite** was watching. A branch switch rewrites
hundreds of source files at once; Vite reacts with **full-page reloads**, and
*mid-checkout* the module tree is briefly inconsistent (imports unresolvable),
so `bootstrap()`'s async bridge handshake can't finish before `initApp`'s 5s
guard throws. Each reload fails the same way → the 385× storm. **The backend
was alive the whole time** — a reload once the tree settled would have
recovered instantly.

### Why this matters beyond dev

The same dead-end appears for any transient bridge hiccup: a stale **pooled
window** whose host IPC port went away, a renderer reload racing host
readiness, or a slow backend spawn (>5s). In every case the backend is usually
reachable seconds later, so the app should **self-heal**, not wedge.

---

## 2. Goals

1. **Self-heal transient failures** — retry the bridge handshake, then do a
   *bounded* full reload, before ever showing a hard error.
2. **No reload storms** — a loop guard so a persistent failure can never repeat
   the 385× behaviour.
3. **Recovery UI** — when auto-heal gives up, show a branded screen with a
   one-click **Reload** (and **Reopen window**) action, a plain-language cause,
   and collapsible technical details — not just "press F12."
4. **Honest status** — during auto-retry, show "Reconnecting… (attempt N/M)" so
   the user isn't staring at a frozen screen.

## 3. Non-Goals

- Fixing Vite's full-reload behaviour itself (it's correct to reload on a mass
  change). The operational lesson — *don't run git branch ops in a clone a dev
  instance is serving* — is documented, not enforced in code.
- Changing the host IPC / pooled-window lifecycle (separate track).

---

## 3.5 Existing precedent — mirror, don't invent

The codebase already solves this exact shape for two adjacent failures; the fix
should reuse their patterns:

- **WebGL context loss** (`frontend/bootstrap.ts:59-73`): a `webglcontextlost`
  listener that reads a **`sessionStorage` reload counter**, auto-reloads after
  1s if under `CONTEXT_LOSS_MAX_RELOADS`, **suppresses** past the cap (logs
  "possible driver issue"), and **resets the counter after 60s** of stability.
  → The bridge-init recovery should be a near-copy of this loop guard.

- **Renderer crash** (`agentmux-cef/src/client/mod.rs:1457`): the host already
  renders "a recovery HTML page that offers Reload / Quit buttons." Two gaps:
  (1) it only triggers on a renderer *crash*, not a frontend JS bridge-init
  failure (which keeps the renderer alive but wedged), and (2) per
  `client/mod.rs:1435`, that page is a `data:` URI where `location.reload()`
  reloads the *wrong* page. The bridge-failure case is different — the document
  is a real `http://localhost:5270/?…` URL, so `location.reload()` **does**
  work and preserves the `__AGENTMUX_IPC_PORT__`/token query params.

**Why `Ctrl+R` did nothing (observed 2026-06-15):** AgentMux's main window is a
CEF host, not a browser — there is **no `Ctrl+R` → reload keybinding** wired
(the host's only reload path is `POST /agentmux/browser/reload` → CDP
`Page.reload`, used for *browser panes*, not the app window). And the WebGL
auto-reload above doesn't fire for a bridge-init failure. So the user's only
recovery was close+reopen — which is the whole motivation for this spec.

## 4. Design

### 4.1 Retry the handshake before failing (`app-init.ts`)

The 5s `window.api` poll currently rejects outright. Replace with a bounded
**retry of the bridge handshake**:

- Poll `window.api` for up to ~8s (cover slow backend spawn — `initCefApi`
  itself waits up to 30s for `backend-ready`, so align the guard or let it run).
- If still unset, treat as a transient and escalate to the reload path (4.2)
  rather than throwing to a dead screen.

### 4.2 Bounded auto-reload with a loop guard

A persistent failure must not reload forever (that *is* the 385× bug). Use a
**time-windowed counter in `sessionStorage`**:

```
key: "amux:bridge-reload-attempts"  →  { count, firstAt }
```

On bridge-init failure:
1. Read the counter. If `count < MAX_RELOADS` (e.g. 3) within a window
   (e.g. 30s), increment, show the "Reconnecting…" overlay (4.3), and
   `location.reload()` after a short backoff (e.g. 800ms × count).
2. If `count >= MAX_RELOADS`, **stop** — clear the counter and render the
   recovery UI (4.4) with reason "auto-recovery gave up after N attempts."
3. On a **successful** bridge init, clear the counter (so a later unrelated
   failure starts fresh).

This bounds the worst case to MAX_RELOADS reloads, never a storm.

### 4.3 "Reconnecting" overlay (transient state)

While auto-reloading, show a minimal centered card over the splash:

```
⟳  Reconnecting to AgentMux…
   attempt 2 of 3
```

so the user sees progress, not a frozen grey screen.

### 4.4 Recovery UI (terminal state) — replaces `showStartupError`

A branded card (reuses app theme tokens), replacing the bare `<div>`:

```
┌────────────────────────────────────────────┐
│  ⚠  AgentMux couldn't connect to its host   │
│                                              │
│  The UI loaded but lost its link to the      │
│  AgentMux host process. This is usually       │
│  temporary — reloading fixes it.              │
│                                              │
│   [ ⟳ Reload ]   [ ⧉ Reopen window ]          │
│                                              │
│   ▸ Technical details                         │
│     [initApp] window.api still undefined…     │
│     Press F12 for the full console.           │
└────────────────────────────────────────────┘
```

- **Reload** → `location.reload()` (primary; clears the loop counter first so
  the manual reload gets a fresh budget).
- **Reopen window** → best-effort: if `window.api` later exists use the host
  "new window" path; otherwise `location.assign(location.pathname +
  location.search)` to re-navigate with the original query params (preserving
  `__AGENTMUX_IPC_PORT__`/token).
- **Technical details** → `<details>` collapsible holding the raw message +
  the F12 hint (today's content), so the surface is friendly by default but the
  detail is one click away.
- Dev-mode line (when `import.meta.env.DEV`): "If you just switched git
  branches in this clone, Vite reloaded mid-change — Reload should recover."

### 4.6 Keyboard reload (complementary)

Because users *expect* `Ctrl+R`/`F5` to reload (and tried it), wire a global
reload keybinding on the **app window** (not just browser panes), so the
keyboard works even when the UI is wedged:

- Frontend: a capture-phase `keydown` listener registered in `bootstrap.ts`
  *before* the bridge handshake (so it survives a wedged init) → `Ctrl+R` /
  `F5` → `location.reload()`.
- This is independent of `window.api`, so it works in exactly the broken state
  where the user needs it most. Loop guard from 4.2 still applies.

### 4.5 API shape

```ts
// error-display.ts
export function showStartupError(message: string, opts?: {
    recoverable?: boolean;     // default true → show Reload/Reopen
    reason?: "bridge" | "window" | "unknown";
}): void;

export function showReconnecting(attempt: number, max: number): void;

// returns true if it scheduled a reload (caller should stop), false if the
// budget is exhausted and the recovery UI was shown instead.
export function tryAutoReload(reason: string): boolean;
```

---

## 5. Files

| File | Change |
|------|--------|
| `frontend/app/init/error-display.ts` | Recovery card + `showReconnecting` + `tryAutoReload` (loop-guarded) |
| `frontend/app-init.ts` | On bridge-init timeout: `tryAutoReload()` instead of bare `showStartupError`; clear counter on success |
| `frontend/app/init/error-display.scss` *(new, optional)* | Card + button styles (or inline, matching current approach) |

## 6. Test / Verify

- Unit: loop guard — N failures → exactly MAX_RELOADS reloads then terminal UI;
  success resets the counter.
- Manual (dev): with a dev instance running, `git checkout` another branch in
  the served clone → confirm the window shows "Reconnecting…", auto-reloads ≤3×,
  and recovers once the tree settles (instead of the 385× dead-screen storm).
- Manual: kill the host IPC (simulate dead bridge) → after the budget, the
  recovery card shows with a working **Reload** button.

## 7. Changeset

`patch` — UX/resilience fix, no schema or API change.

## 8. Operational note (not code)

Don't run `git checkout` / `rebase` / `pull` in a clone a dev instance is
actively serving — use a separate worktree (`git worktree add`) or stop
`task dev` first. This spec makes that mistake *recoverable*, not impossible.
