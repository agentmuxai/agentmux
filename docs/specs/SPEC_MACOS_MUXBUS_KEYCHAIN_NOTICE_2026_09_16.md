# SPEC: Warn macOS users up front that MuxBus sign-in will prompt for Keychain access

**Date:** 2026-09-16
**Status:** implemented — agentmux side in PR #3271 (approved); agentmux-cloud
companion side in `agentmux-cloud` PR #75 (review in progress, addressing the
iOS/iPadOS UA false-positive and package.json version bump).
**Repos touched:** `agentmuxai/agentmux` (desktop app) and `agentmuxai/agentmux-cloud`
(hosted login-relay page). Cross-repo specs are kept in `agentmux`'s `docs/specs/`
per existing precedent (see `SPEC_MUXBUS_CLOUD_RELAYED_LOGIN_CALLBACK_2026_08_15.md`,
referenced from `agentmux-cloud/muxbus/server/src/login-relay.ts`).
**Related:** `docs/retro/retro-macos-muxbus-keychain-prompt-storm-2026-08-19.md` —
background on *why* macOS shows this prompt at all.

---

## 0. Motivation

Signing in to MuxBus Cloud on macOS ends with the OS's native Keychain
access-consent dialog ("AgentMux wants to use your confidential information
stored in Keychain…"). Today nothing tells the user this is coming. The retro
above already reduced this to **one** dialog instead of up to twelve, but one
unexplained system dialog appearing right after a web sign-in still reads as
suspicious/interruptive if the user isn't expecting it. This spec adds a short,
macOS-only note at the two points in the flow where a user commits to signing
in, so the dialog is expected rather than alarming.

Not in scope: changing *when* or *how many times* the prompt fires (that's the
retro's territory), or building a native OS desktop notification — "desktop
notification" here refers to the hosted web page shown to the user's system
browser during the flow, not a push/toast notification.

## 1. Why the Keychain prompt happens (background)

MuxBus Cloud is a single, app-wide session (not a pluralizable identity
account). Its tokens are persisted via `agentmux-srv/src/identity/secret_store.rs`,
which backs onto the OS keychain (macOS Keychain / Windows Credential Manager /
Linux Secret Service) through the `keyring` crate. On macOS specifically, the
first `secret_store::put`/`get` for the `agentmux` service account triggers the
OS's own access-consent UI — this happens once the browser hands control back
to the desktop app and it persists the freshly-obtained token
(`agentmux-srv/src/backend/storage/muxbus.rs`'s `muxbus_save` path, ultimately
`secret_store::put(LEGACY_BLOB_KEYCHAIN_ID, &blob)`). Windows/Linux don't show
an equivalent interactive OS dialog in the same way, so this notice is
deliberately macOS-only.

## 2. Two touchpoints

### 2.1 Status bar panel — "Sign in" button (agentmux repo)

`frontend/app/statusbar/HostPopover.tsx`, the "MuxBus Cloud" row (~lines
307–350) renders a **Sign in** button when not connected:

```tsx
<Show when={muxbus.isConfigured()}>
    <div class="status-bar-popover-divider" />
    <div class="status-bar-popover-row">
        <span class="status-bar-popover-label">MuxBus Cloud</span>
        <Show
            when={muxbus.status()?.connected && muxbus.status()?.valid}
            fallback={
                <button ... onClick={...}>
                    {muxbus.loading() ? "Cancel" : muxbus.status()?.connected ? "Expired — re-login" : "Sign in"}
                </button>
            }
        >
            ...
        </Show>
    </div>
    ...
</Show>
```

This is the app's own live platform, not a browser — platform detection is
already solved: `isMacOS()` from `frontend/util/platformutil.ts` (already used
elsewhere in this file's sibling components, e.g. `tabbar.tsx`,
`hamburger-menu.tsx`).

**Change:** add a one-line note directly under the button row, shown only
when `isMacOS()` is true and the fallback (not-yet-signed-in) branch is
rendering:

```tsx
<Show when={isMacOS() && !(muxbus.status()?.connected && muxbus.status()?.valid)}>
    <div class="status-bar-popover-row status-bar-popover-hint">
        macOS will ask for Keychain access after you sign in — that's AgentMux
        securely storing your session.
    </div>
</Show>
```

(Exact class name to match existing hint/note styling in this file —
`status-bar-popover-hint` is a placeholder; reuse whatever class the
`⚠ {muxbus.error()}` row already uses for a muted secondary line, or add a
new minimal style if none fits.)

The same `MuxBusController`-driven "Sign in" / "Connect" affordance also
appears in two more places that share this component
(`frontend/app/view/accounts/AgentMuxConnectPanel.tsx`'s
`MuxBusConnectSection` — the per-agent Identity tab — and
`AgentMuxConnectPanel` — the Armory → Accounts gallery tile). The user's ask
was specifically the status bar panel; whether to add the same note to those
two is an open question (§4).

### 2.2 Hosted login-relay page (agentmux-cloud repo)

`muxbus/server/src/login-relay.ts`'s `desktopCallbackHtml()` is the page
Cognito redirects the user's **system browser** to after they authenticate
(`GET /desktop-callback` — see `SPEC_MUXBUS_CLOUD_RELAYED_LOGIN_CALLBACK_2026_08_15.md`
for the full relay design). It's a dependency-free inline HTML/JS string
served by the relay Lambda; it forwards `{state, code}` to
`/api/login-relay/submit` and then shows a success message:

```html
<h1 id="title">Finishing sign-in…</h1>
<p id="detail">Handing the sign-in result back to AgentMux.</p>
...
title.textContent = "Connected to AgentMux Cloud";
detail.textContent = "You can close this tab and return to AgentMux.";
```

This runs entirely client-side in whatever browser the OS opened, so platform
detection has to be **browser-side**, not server-side — the relay Lambda
itself has no reliable signal for the *desktop OS* (only whatever the
browser's own `User-Agent` request header says, which is one more hop of
indirection than just reading it in the page's own JS). `navigator.userAgent`
(or `navigator.platform`, though that's been deprecated in some browsers) is
sufficient and is exactly what's available to this page:

```js
const isMac = /Macintosh/.test(navigator.userAgent) && !/iPhone|iPad|iPod/.test(navigator.userAgent);
```

(A bare `/Mac/` test is NOT sufficient — iPhone/iPod UAs contain the substring
"like Mac OS X", so they'd false-positive on the note even though they never
show a native macOS Keychain prompt. Real desktop Safari/Chrome/Firefox on
macOS carries "Macintosh"; iOS/iPadOS mobile UAs don't. This does not
disambiguate iPadOS *requesting desktop-class UA* — as of recent iPadOS
versions Safari there reports an indistinguishable "Macintosh; Intel Mac OS X"
string by design — but that's an accepted, low-stakes edge case: worst case a
Keychain hint shows on an iPad that never sees the prompt, ReAgent-reviewed
and judged not worth further complexity. Caught in review on the
`agentmux-cloud` PR — see PR #75.)

**Change:** on success, append a short macOS-only line to the detail text:

```js
title.textContent = "Connected to AgentMux Cloud";
detail.textContent = "You can close this tab and return to AgentMux.";
if (/Macintosh/.test(navigator.userAgent) && !/iPhone|iPad|iPod/.test(navigator.userAgent)) {
  detail.textContent += " macOS may ask for Keychain access — that's AgentMux securely storing your session.";
}
```

No new dependency, no build step — `desktopCallbackHtml()` is a plain
template string returned by a Lambda handler, matching the file's existing
"deliberately dependency-free inline HTML" design note.

## 3. Copy

Keep both notices short and consistent so they read as the same feature:

- Status bar panel (before sign-in): *"macOS will ask for Keychain access
  after you sign in — that's AgentMux securely storing your session."*
- Login-relay page (after sign-in succeeds): *"macOS may ask for Keychain
  access — that's AgentMux securely storing your session."*

("will" vs. "may" is intentional: the status-bar note is shown before the
user has committed, describing what's about to happen; the relay page's note
fires after the token exchange, where the prompt may already have been
answered by the time the user reads it, or may still be pending back on the
desktop side.)

## 4. Open questions

- Should the same note also appear in `MuxBusConnectSection` (Identity tab)
  and `AgentMuxConnectPanel` (Armory Accounts gallery) — the two other
  surfaces that render an equivalent "Connect"/"Sign in" button via the same
  `MuxBusController`? They share `useMuxBusStatus()`, so the `isMacOS()` gate
  would be identical; only the JSX insertion point differs per component.
- Exact CSS class for the hint line in `HostPopover.tsx` — needs a look at
  `_statusbar.scss` (or wherever this file's styles live) to match existing
  muted/secondary-text conventions rather than inventing a new one.
- Whether the login-relay note should be worded differently if `idpError` or
  the failure branch fires — currently scoped to the success path only, since
  a failed sign-in never reaches the point of writing to Keychain.
