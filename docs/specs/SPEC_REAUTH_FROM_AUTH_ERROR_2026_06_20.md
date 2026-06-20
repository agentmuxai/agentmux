# SPEC: Re-authentication from Agent Auth Failure
**Date:** 2026-06-20  
**Author:** AgentA  
**Status:** Draft  
**Depends on:** SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16.md §P1.1–P1.3 (merged #1589/#1590)

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
    │ 3. open ReauthBrowserModal (CEF browser pane, see §6)         │
    │    - loads auth_url in embedded browser                       │
    │    - shows URL as copyable fallback below viewport            │
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

## 6. ReauthBrowserModal

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
auth_url present → ReauthBrowserModal
auth_url absent  → openExternal fallback (existing path)
```

No backend changes required for this spec.

---

## 9. Implementation Plan

### P2.1 — ReauthBrowserModal component
- New file: `frontend/app/view/agent/components/ReauthBrowserModal.tsx`
- Wraps the existing CEF browser widget in a modal (reuse `ModalLayer`)
- Props: `{ providerName: string; authUrl: string; onClose: () => void; onSuccess: () => void }`
- Browser viewport via `<BrowserPane>` or equivalent CEF embedded renderer
- URL fallback bar + status line

### P2.2 — Launch-flow auth branch update
- `launch-flow.ts`: after `auth_url` is captured, if present open `ReauthBrowserModal` instead of `openExternal()`
- The existing polling loop and success/timeout handling remain; they call modal `onSuccess` / allow `onClose`
- Gate on a `isReauthContext: boolean` param so the first-launch flow (pre-launch OAuth modal) is unchanged

### P2.3 — Inline error node CTA
- `DocumentRow.tsx`: for `agent_error` nodes with code 401/403, render a `[Login Again →]` button
- The button calls the `onLoginAgain` prop threaded from `AgentPresentationView`
- Thread `onLoginAgain` down through `DocumentRowProps` (add optional prop)

### P2.4 — Tests
- `ReauthBrowserModal` unit test: renders URL fallback bar, calls `onSuccess` on signal, calls `onClose` on ✕
- `launch-flow` test: with `isReauthContext: true` and a mock `auth_url`, verify modal opens (not `openExternal`)
- `DocumentRow` test: code 401 renders CTA button; code 200 does not

---

## 10. Open Questions

| # | Question | Default |
|---|---|---|
| Q1 | Should the CEF browser in the modal share cookie store with the main window, or use an isolated profile? | Isolated — same policy as BrowserPane. |
| Q2 | If the user completes auth in the system browser (old path from "Open in browser"), does poll detect it? | Yes — poll calls `CheckCliAuthCommand` which reads credentials on disk; it doesn't care which browser set them. |
| Q3 | Modal width/height: 480×580 enough for Claude's consent page? | Needs a smoke test against actual claude.ai/oauth page. Adjust in P2.1. |
| Q4 | Should auth modal be dismissible by clicking outside? | No — accidental dismissal during OAuth redirect would lose the flow. Only [✕] and success close it. |
