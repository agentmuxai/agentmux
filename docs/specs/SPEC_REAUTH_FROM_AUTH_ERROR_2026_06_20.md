# SPEC: Re-authentication from Agent Auth Failure
**Date:** 2026-06-20  
**Author:** AgentA  
**Status:** In progress (P2.3 merged #1592; P2.1 in PR)  
**Depends on:** SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16.md §P1.1–P1.3 (merged #1589/#1590)

> **Revision 2026-06-20 — "ReauthBrowserModal" replaced by an in-app browser
> pane.** The original draft (§6) proposed embedding a CEF browser inside a
> modal overlay. A codebase study found this is **architecturally impossible**:
> a browser pane is a real native HWND child window positioned in physical
> pixels synced to the page layout, and cannot be CSS-positioned inside a modal.
> The idiomatic in-app "browser window" is a **browser pane** created via
> `createBlock({ meta: { view: "browser", url } })`. §6 below is superseded by
> **§6′ (In-app browser pane)**; the rest of the spec stands.

---

## 1. Problem

When an agent hits a 401/auth error the user now sees it clearly (P1.3 inline error node, P1.1/P1.2 failure banner). But clicking **Login Again** currently re-runs the full launch flow — `startLaunchFlow()` — which calls `openExternal()` to punt the OAuth URL to the system browser. The result is:

- System browser opens instead of AgentMux's own browser → context switch, user may not notice
- The URL is shown in a small text box above the composer → easy to miss
- If the system browser doesn't open (sandbox, missing default browser, kiosk), nothing happens
- No feedback while waiting for auth to complete

**Goal:** When the user clicks **Login Again** from an auth failure, AgentMux opens the provider's OAuth flow in a first-class way — for Claude a CEF browser window with the URL shown as a fallback — and closes it automatically on success.

---

## 2. Scope

| In scope | Out of scope |
|---|---|
| Claude provider re-auth on auth failure | New-identity creation (handled by pre-launch OAuth modal — SPEC_PRE_LAUNCH_OAUTH_FLOW) |
| Other providers via their auth method | Gemini-specific auth fix (#1250 — separate issue) |
| CTA from failure banner and inline error node | API-key-only providers (no URL to open) |
| URL fallback display | Trust Center binding (SPEC_TRUST_CENTER_CLI_AUTH_BINDING) |

Triggers considered:
- Failure banner "Login Again" action (already wired; behavior changes)
- Inline `agent_error` node CTA button (new in this spec)

---

## 3. Trigger Conditions

Re-auth flow is triggered when ALL are true:

1. `agentFailure.code === "auth_failure"` **or** the inline `AgentErrorNode` has `code` 401 or 403
2. User clicks **Login Again** from either the banner or the inline error node

The two surfaces share one handler — `onLoginAgain` in `agent-view.tsx`.

---

## 4. Current Flow (Baseline)

```
onLoginAgain
  → status.startLaunchFlow()
    → Phase 2: runCliLogin()
      → spawn: claude auth login
      → capture auth_url from stdout (2 s window)
      → openExternal(auth_url)          ← punts to system browser
      → show URL in auth-url box above composer
      → poll CheckCliAuthCommand every 2 s until success or 5 min timeout
    → Phase 3: ControllerResyncCommand  ← agent restarts
```

---

## 5. New Flow

```
onLoginAgain
  → dispatch: provider.reauthInPlace(ctx)
    ┌─ Claude provider ─────────────────────────────────────────────┐
    │ 1. spawn: claude auth login (same as before)                  │
    │ 2. extract auth_url from stdout                               │
    │ 3. openOAuthBrowserPane(auth_url)  ← in-app pane, see §6′      │
    │    - createBlock browser pane beside the agent                │
    │    - AuthUrlBox above composer shows URL as backup            │
    │ 4. poll CheckCliAuthCommand every 2 s                         │
    │ 5. on success:                                                │
    │    - close modal                                              │
    │    - show brief "Authenticated" toast                         │
    │    - ControllerResyncCommand → agent restarts                 │
    │ 6. on timeout (5 min) or user closes:                         │
    │    - close modal                                              │
    │    - leave failure banner/error node visible                  │
    └───────────────────────────────────────────────────────────────┘
    ┌─ Other providers (generic) ───────────────────────────────────┐
    │ Same path — if auth_url captured: open ReauthBrowserModal     │
    │ If no URL (e.g. device flow, API key only): fall back to      │
    │ existing openExternal() + URL-box behavior (no regression)    │
    └───────────────────────────────────────────────────────────────┘
```

Key differences from baseline:
- `openExternal()` is replaced by `ReauthBrowserModal` for providers that return a URL
- The URL backup is shown inside the modal, not in the composer area
- Success auto-closes the modal and restarts the agent
- Timeout/cancel path is explicit, leaves failure state intact

---

## 6′. In-app browser pane (supersedes §6)

On re-auth, after `runCliLogin()` captures `auth_url`, open it in an **in-app
browser pane** instead of the system browser:

```
openOAuthBrowserPane(url):
  try    createBlock({ meta: { view: "browser", url } })   → "pane"
  catch  getApi().openExternal(url)                        → "external"
  catch  (nothing opened)                                  → "failed"
```

- `createBlock` splits the **current tab** (magnified = false) so the browser
  pane appears beside the agent pane — the agent's **AuthUrlBox** (URL text +
  copy + paste-the-code input) stays visible, which the user needs for
  providers that return a code to paste rather than redirecting.
- `openExternal` (system browser) is the **fallback** if the pane can't be
  created (no layout model, RPC error). Never throws — login UX degrades, never
  crashes the launch flow.
- The polling / success / timeout machinery in `launch-flow.ts` is unchanged;
  only the "how the URL opens" step changes.
- This improves **all** CLI logins (first launch + re-auth) since both share
  `launch-flow.ts` — no re-auth-only gating needed.

**AuthUrlBox** gains an **"Open"** button (alongside Copy) that re-opens the URL
in an in-app pane on demand — the explicit "browser window" affordance if the
auto-open was missed. The URL text + paste-code input remain the backup.

Why not the alternatives (from the codebase study):
- **Embedded-in-modal** — impossible; CEF pane needs a native HWND, can't be
  CSS-positioned in a modal overlay.
- **`openNewWindow()`** — opens a whole AgentMux workspace window (no URL param);
  overkill and confusing for OAuth.
- **`openExternal()` only** — leaves the app; kept only as the fallback.

---

## 6. ReauthBrowserModal — SUPERSEDED (see §6′)

> **Not implemented.** Kept for historical context. A modal-embedded CEF browser
> is architecturally impossible in this codebase; §6′ is the shipped design.

A modal dialog with an embedded CEF browser viewport. Reuses the existing browser-pane infrastructure.

### Layout

```
┌─────────────────────────────────────────────────────┐
│  Log in to Claude                              [✕]  │
├─────────────────────────────────────────────────────┤
│                                                     │
│   [  CEF browser viewport — loads auth_url  ]       │
│   [  (https://claude.ai/oauth/authorize/…)  ]       │
│   [                                         ]       │
│   [  height: ~540px, width: ~480px          ]       │
│                                                     │
├─────────────────────────────────────────────────────┤
│ If the browser doesn't load, open this URL:         │
│ https://claude.ai/oauth/… [Copy]  [Open in browser] │
└─────────────────────────────────────────────────────┘
```

- Title: `"Log in to <provider name>"` (e.g. "Log in to Claude")
- Browser viewport occupies the modal body
- URL fallback bar is always visible at the bottom (not hidden behind a toggle)
- "Open in browser" calls `openExternal()` — fallback to old behavior
- [✕] closes the modal and cancels auth (leaves failure state visible)
- Modal is non-resizable; fixed dimensions chosen to fit Claude's consent page

### While waiting

A spinner overlay covers the browser until the URL loads. After load, spinner disappears.

A status line at the bottom (above the URL bar) shows:
- While polling: "Waiting for authentication…"
- On success: "Authenticated ✓" (1.5 s, then modal closes)
- On timeout: "Login timed out. Try again." (modal stays open, user can close)

### Failure to extract URL

If `auth_url` is `null` (CLI didn't print a URL within the capture window):
- Skip the modal entirely
- Fall back to current behavior: show a "Run `/login` to authenticate" instruction in the failure banner
- Log a warning so we know the capture window may need tuning

---

## 7. Inline Error Node CTA

The `AgentErrorNode` rendered by `DocumentRow.tsx` currently shows code + message only (P1.3). For `code === 401` or `code === 403`, add a CTA button:

```
┌─────────────────────────────────────────────────────┐
│ [!] HTTP 401  Failed to authenticate. API Error:    │
│              401 Invalid authentication credentials  │
│                                    [Login Again →]  │
└─────────────────────────────────────────────────────┘
```

- Button only rendered when `node.code === 401 || node.code === 403`
- Calls the same `onLoginAgain` handler the failure banner uses
- Styled as a small secondary button, right-aligned inside the error block

This gives the user two entry points to re-auth — the persistent failure banner and the inline node — with identical behavior.

---

## 8. Provider Contract

Each provider definition (`frontend/app/view/agent/providers/`) already has `authLoginCommand` and `requiresLoginTty`. No new fields needed — the existing `runCliLogin()` RPC returns `auth_url: Option<String>`, which drives the branching:

```
auth_url present → openOAuthBrowserPane (in-app pane, system-browser fallback)
auth_url absent  → existing "run login manually" warning path
```

No backend changes required for this spec.

---

## 9. Implementation Plan

### P2.1 — In-app browser pane on re-auth ✅ (this PR)
- New file: `frontend/app/view/agent/flows/open-oauth-pane.ts` — `openOAuthBrowserPane(url)`
  returns `"pane" | "external" | "failed"` (createBlock → openExternal → nothing).
- `launch-flow.ts`: replace the `openExternal(loginUrl)` call with `openOAuthBrowserPane(loginUrl)`;
  log which path opened. No `isReauthContext` gate needed — improving all logins is desired.
- `AgentDocumentView.tsx` AuthUrlBox: add an **"Open"** button that re-opens the URL in a pane.
- Tests: `open-oauth-pane.test.ts` — pane success, external fallback, failed (no throw).

### P2.2 — (folded into P2.1)
The original P2.1/P2.2 split assumed a modal component + a flow branch. With the
pane approach there's no component to build — the flow change and the helper are
one small PR.

### P2.3 — Inline error node CTA ✅ (merged #1592)
- `DocumentRow.tsx`: for `agent_error` nodes with code 401/403, render a `[Login Again →]` button.
- Threaded `onAgentErrorLogin` from `agent-view.tsx` → `AgentDocumentView` → `AgentDocumentVirtualList` → `DocumentRow`.
- Tests in `DocumentRow.test.tsx`.

### P2.4 — Tests ✅ (with each slice)
- `DocumentRow.test.tsx` (P2.3), `open-oauth-pane.test.ts` (P2.1).
- A full `launch-flow` integration test for the open-branch is deferred — the helper is unit-tested
  and the branch is a one-line swap.

---

## 10. Open Questions

| # | Question | Default |
|---|---|---|
| Q1 | Does the in-app browser pane share the cookie store with the main window? | Yes — browser panes use the shared CEF profile, so a prior claude.ai login carries over (smoother consent). |
| Q2 | If the user completes auth in the system browser (fallback path), does poll detect it? | Yes — poll calls `CheckCliAuthCommand` which reads credentials on disk; it doesn't care which browser set them. |
| Q3 | The OAuth pane splits the current tab — does it crowd the agent pane? | Acceptable: the split keeps both visible so the user can paste the code back. The pane can be closed/redocked normally after login. A future refinement could open it as an ephemeral/magnified pane that auto-closes on success. |
| Q4 | Should the pane auto-close on auth success? | Not in this PR — the launch-flow doesn't track the created blockId. Tracked as a follow-up; closing it is a normal pane action meanwhile. |
