# Retro — "Failed to authenticate" + dead "Login Again" button on a new agent pane

**Date:** 2026-07-01
**Severity:** High (user-facing dead end — an agent pane can't recover from an auth lapse via the
primary button offered)
**Status:** Root-caused (ranked); fix not yet implemented
**Reporter:** asaf
**Component:** agent pane auth / re-auth flow (frontend `view/agent`, srv `identity`/`agents`)

---

## 1. What happened

Opening a **new AgentA agent pane (Claude)** surfaced **"Failed to authenticate"**, and clicking
the offered **"Login Again"** button **did nothing** — no browser, no console, no visible error, no
state change.

Two distinct defects:
- **A —** the pane's Claude CLI returned a 401 (auth failure) on first use.
- **B —** the recovery affordance ("Login Again") is a **silent no-op** for this Claude CLI version.

B is the more serious bug: the user is shown a recovery button that cannot recover.

---

## 2. How the flow is supposed to work

1. The agent CLI (claude) runs in the pane. On a bad/expired token it prints
   `Failed to authenticate. API Error: 401 Invalid authentication credentials`.
2. srv detects this via `identity/auth_patterns.rs` (`"failed to authenticate"`, `auth_patterns.rs:157`)
   → classifies it (`agents/failure.rs`) → the frontend renders the auth-recovery row with actions:
   **Login Again**, **Use existing login**, **Login via terminal** (`agent-view.tsx:777-795`).
3. **Login Again** → `status.relogin()` (`useAgentControllerStatus.ts:171`) →
   `forceProviderLogin()` (`flows/force-login.ts:43`), which is *designed to bypass* the gated
   auth-status check because a 401 is an "expired-but-present" false positive that the gated launch
   flow would trust and skip (see the comment at `agent-view.tsx:767-776`,
   `SPEC_REAUTH_FROM_AUTH_ERROR §11`).

The design already anticipates the "check says present, but it's expired → do nothing" trap. The bug
is that **the re-auth path has its own way of doing nothing.**

---

## 3. Root-cause analysis (ranked)

### H1 (primary) — `forceProviderLogin` silently no-ops for Claude Code v2.1.x's un-scrapeable login TUI
`force-login.ts` does:
```ts
const url = await getApi().runCliLogin(...);   // scrape the OAuth URL from the CLI
if (url) { setAuthUrl(url); openOAuthBrowserPane(url); ... }
else {
    // "No URL captured (some providers don't print one); the CLI may have
    //  opened its own login TUI."
    log("auth", "a browser window should have opened — complete login there", "warn");  // ← warn only
}
```
For **Claude Code v2.1.x**, `runCliLogin` returns **no scrape-able URL** — the CLI moved to an
interactive login TUI that prints no parseable URL (the codebase itself documents this:
`agent-view.tsx:782-784` calls "Use my existing login" *"the reliable path for Claude Code v2.1.x's
un-scrapeable login TUI"*). So `forceProviderLogin` falls into the `else` branch, logs a **WARN**,
and **nothing opens** — no browser, no console, no error surfaced to the user. From the user's seat,
"Login Again" is dead. **This matches the report exactly.**

### H2 (secondary) — `relogin`'s `cliPath`-missing fallback degrades to the gated no-op
`relogin()` (`useAgentControllerStatus.ts:183-186`): if the block's `cmd` meta (`cliPath`) isn't
resolved, it **falls back to `startLaunchFlow()`** — the *gated* flow that the whole re-auth feature
exists to bypass (it trusts `CheckCliAuth`'s expired-but-present false positive and skips login).
So in the `cliPath`-unresolved case, "Login Again" degrades into precisely the no-op it was built to
avoid. Whether this fires depends on block-meta timing on a freshly-opened pane.

### H3 (tertiary) — `provider()` null / button not wired
`relogin()` early-returns with a WARN if `opts.provider()` is null (`:174-177`). And the click only
reaches `relogin` if `onLoginAgain` is actually bound to the rendered button. **Diagnostic to
disambiguate H1/H2 from H3:** check the frontend console log for the line
`"Login Again — forcing a fresh provider login"` (`agent-view.tsx:778`) at click time:
- **present** → the click fired; failure is downstream (H1/H2). Look for the next line
  `"re-login: forcing a fresh OAuth…"` and whether a URL followed.
- **absent** → the click never reached the handler (H3 — wiring/event issue).

> Note: a scan of the live session's `srv-events.log` showed only this debugging session's own
> `term:activity` markers, not the auth flow — the auth/relogin lines are **frontend console logs**
> (`[fe] [auth] …`), which land in the CEF sidecar / `cef-debug.log`, not `srv-events.log`. Pulling
> that specific stream at repro time is the fastest confirmation.

### Symptom A root — the underlying 401
The per-agent isolated auth dir (per-agent identity provisioning, #1858) held an **expired or unseeded**
credential. `CheckCliAuth` reports it *present* (file exists) but not *valid*, so the gated launch
trusted it and didn't prompt — the pane started, then 401'd on first request. This is the
"expired-but-present false positive" the re-auth feature names.

**Most likely combined cause: A (expired per-agent credential) → B/H1 (fresh-OAuth re-auth can't
produce a URL for Claude Code v2.1.x, fails silently).**

---

## 4. Contributing factors
- **External CLI UX change.** Claude Code v2.1.x replaced the URL-printing login with an
  un-scrapeable TUI. The re-auth flow's "scrape the OAuth URL" assumption silently broke against it.
  The scrape approach is inherently brittle to CLI UX changes.
- **Silent failure.** The no-URL branch logs a `warn` and shows **no user-visible error/toast** — so a
  broken path is indistinguishable from a dead button.
- **Wrong affordance foregrounded.** The codebase already knows the reliable paths for Claude v2.1.x
  are **"Use existing login"** (seed-from-global) and **"Login via terminal"** (real console) — yet
  **"Login Again"** (the unreliable fresh-OAuth path) is presented as a peer/primary action, and it's
  the one a user naturally clicks.
- **`relogin` fallback undermines its own purpose** (H2) — degrading to the gated flow re-introduces
  the exact bug the feature was built to fix.

---

## 5. Recommended fixes (ranked)
1. **Never fail silently.** In `force-login.ts`, when `runCliLogin` returns no URL, surface a
   **user-visible error** ("Couldn't start a browser login for this CLI version — use *Use existing
   login* or *Login via terminal*"), not just a `warn`. This alone turns a dead button into a guided
   recovery. *(Small, high-value.)*
2. **Route "Login Again" to a reliable path for Claude Code v2.x.** When the provider is Claude Code
   and the fresh-OAuth scrape is known-unreliable, either (a) make "Login Again" perform the
   terminal-login / seed-from-global path, or (b) disable/de-emphasize it and promote the two reliable
   actions. Don't offer a button that structurally cannot work for the active provider.
3. **Fix the `relogin` `cliPath`-missing fallback (H2).** Don't degrade to the gated `startLaunchFlow`
   — that path no-ops on the false positive. Resolve `cliPath` (or run the forced login) instead.
4. **Address the root 401 (A).** Make `CheckCliAuth` detect *expired-but-present* (validate, don't
   just stat the file), so a bad per-agent credential prompts a real login at launch instead of
   starting and 401'ing.

## 6. Prevention
- **Pin/track Claude Code CLI version compatibility** for the login-scrape; add a guard/warning when
  the installed CLI version is outside the tested range for OAuth-URL scraping.
- **Unit-test the no-URL branch** of `forceProviderLogin` (asserts a visible error is produced).
- **E2E the re-auth flow** end to end (401 → auth row → each recovery action produces a visible
  outcome), so a silently-dead recovery button fails CI, not the user.
- **Log the decision, not just the attempt** — `relogin` should log which branch it took (forced /
  fallback / no-provider) so this is diagnosable from logs without a repro.

## 7. Diagnostic to confirm at next repro
1. Repro the auth failure on a fresh Claude pane.
2. Click "Login Again"; capture the `[fe] [auth]` console lines.
3. Confirm sequence: `Login Again — forcing a fresh provider login` → `re-login: forcing a fresh
   OAuth…` → (expected) **no URL** → `a browser window should have opened` warn with nothing opening
   ⇒ confirms **H1**. If the first line is absent ⇒ **H3** (wiring). If `CLI not resolved` appears ⇒
   **H2**.

## 8. References
- Frontend: `view/agent/agent-view.tsx:758-796` (auth actions), `:1070-1071` (inline error node);
  `view/agent/hooks/useAgentControllerStatus.ts:171-205` (`relogin`), `:207-` (`useGlobalLogin`);
  `view/agent/flows/force-login.ts:43-` (`forceProviderLogin`).
- srv: `identity/auth_patterns.rs:150-157,333` (failure detection), `agents/failure.rs:516`.
- Specs: `SPEC_REAUTH_FROM_AUTH_ERROR` (§11), `SPEC_HOST_CLI_LOGIN_CAPTURE` (§5.5),
  `SPEC_PROVIDER_ISOLATION` (§4.5); related: per-agent identity provisioning (#1858), IdentityLink (#1852).
