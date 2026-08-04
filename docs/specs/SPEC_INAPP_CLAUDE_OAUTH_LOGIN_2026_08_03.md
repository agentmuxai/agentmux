# SPEC — In-app (no-shell) Claude OAuth login, revived, at all three auth surfaces

**Date:** 2026-08-03
**Author:** Nark
**Status:** PROPOSED
**Supersedes/updates:** the "in-app OAuth is a DEAD END for Claude v2.1.x" verdict in `frontend/app/view/agent/providers/catalog.ts` and `SPEC_HOST_CLI_LOGIN_CAPTURE_2026_06_20.md` §0's abandonment note; extends `REPORT_AUTH_ARCHITECTURE_STATE_AND_RETHINK_2026_07_21.md` §8 with new evidence.
**Related:** `PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md` (single-point enforcement stays; this spec fills the "richer per-account Connect UX" follow-up it deferred).

---

## 1. Problem

Logging a Claude agent in today is unreliable and console-bound. The current tiered flow (`frontend/app/view/agent/flows/run-provider-login.ts`) skips tier 1 (in-app URL capture + paste-code) entirely for Claude — `skipTier1: true` at all call sites, `headlessLoginUrlUnsupported: true` in the provider catalog — leaving only:

- **Tier 2** (`seedGlobalLogin`): only works if a valid global `~/.claude` login already exists.
- **Tier 3** (`openLoginTerminal`): opens a real terminal window and polls for up to 5 minutes. Requires a working shell/PTY/terminal stack, is visually jarring, and fails opaquely on headless or misconfigured hosts.

Meanwhile the single-point-login spawn gate (`agentmux-srv/src/identity/resolver/inject.rs`) hard-blocks any Claude agent with no bound account — correct policy, but with no in-app way to create that account, users hit a dead-end "Agent encountered an error" (see `retro-agentu-0.54.9-stuck-error-2026-08-03`, the v0.54.9 stuck-instance incident).

An in-app paste-code login UI **already exists and still works** — `AuthUrlBox` (`frontend/app/view/agent/components/AgentDocumentView.tsx:219-346`), fed by IPC `set_provider_auth` → `CliLoginStdin::write_line` (`agentmux-cef/src/commands/providers.rs:329`). It was built in #1277 and cut off for Claude by `35af4958f` because Claude Code **v2.1.183** never printed a login URL when host-spawned. That factual basis is now stale.

## 2. New evidence (2026-08-03, live probes)

Verified against both the AgentMux-pinned CLI (**2.1.198**, `instances/v0.54.6/cli/claude` and `v0.54.9`) and a current global install (**2.1.214**):

1. `claude auth login` (new `auth` command group; also `claude setup-token`), spawned under a PTY with an isolated `CLAUDE_CONFIG_DIR` and **no `DISPLAY`**, prints the **full PKCE authorize URL** as a fallback:
   `If the browser didn't open, visit: https://claude.com/cai/oauth/authorize?code=true&client_id=…&code_challenge=…&code_challenge_method=S256&state=…`
2. It then presents a stdin prompt: `Paste code here if prompted >` — exactly the input the existing `set_provider_auth` plumbing delivers.
3. If the user completes authorization in a browser, the CLI **detects completion on its own** (polling keyed by `state`) and finishes without any paste — verified end-to-end: a real credential was minted with zero terminal interaction beyond spawning the process.
4. `claude auth login` flags: `--claudeai` (default, subscription), `--console` (API billing), `--sso`, `--email <email>`. Scopes requested: `org:create_api_key user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload`. `claude setup-token` requests only `user:inference` and prints a 1-year `sk-ant-oat01-…` token (`CLAUDE_CODE_OAUTH_TOKEN`).

Consequences:
- Tier 1 in-app capture is **viable again** for Claude — and better than before: the happy path needs no paste at all.
- Anthropic's lack of a device-authorization endpoint (auth-broker Phase D conclusion, still true) is **irrelevant**: the CLI itself is the OAuth client; we only need to relay its URL and (optionally) its code prompt.

## 3. Design

### 3.1 Core: an "in-app login session" primitive

One reusable flow (backend-spawned, frontend-observed), extracted so all three surfaces share it:

- Spawn `<pinned claude> auth login` under a PTY with `CLAUDE_CONFIG_DIR=<isolated per-account dir>` (minted the same way tier 2/3 mint account dirs today) and **no browser-launch capability suppressed** — let the CLI try `xdg-open`/`open` first; if the user's desktop browser opens, that's the fastest path.
- Scrape stdout for the authorize URL (reuse/extend the existing tier-1 `forceProviderLogin` capture machinery; the URL is OSC-8 hyperlinked and printed plainly — match `https://claude.com/cai/oauth/authorize?…`).
- Emit states to the frontend: `spawning → url_captured(url) → waiting_for_authorize → completed | failed(reason)`.
- **Completion detection:** CLI exit 0 **plus** credential material present in the isolated dir (same check `pollForGlobalLoginSeed`/auth-status probing uses). No fixed 5-minute terminal-poll window; the session lives as long as the panel is open, with a cancel affordance.
- **Paste path:** if the user's browser lands on the code-display page (e.g. app-opened browser on another machine profile, or auto-poll fails), the existing `AuthUrlBox` paste input → `set_provider_auth` → CLI stdin completes it.
- On success: register the `IdentityAccount` row with `SecretRef::OAuthConfigDir { dir }` and (surface-dependent) link the agent — identical persistence to today's tier 2/3 (`identity_auth_persist.rs`), no new credential storage semantics.

### 3.2 Catalog/flag changes

- `frontend/app/view/agent/providers/catalog.ts`: drop `headlessLoginUrlUnsupported: true` for Claude; add the login argv (`["auth", "login"]`) alongside the existing login metadata. Keep `requiresLoginTty` semantics for providers that truly need it.
- Remove the three `skipTier1: true` overrides (`useAgentControllerStatus.ts`, `commands/global/login.ts`, `PreLaunchAuthPanel.tsx`) for Claude; tier 1 becomes the revived in-app session (§3.1). Tier 2 (seed-from-global) stays as the fast path when a valid global login exists. Tier 3 (terminal) demotes to explicit fallback ("Use terminal instead"), never auto-launched.
- **Feature-gate, not version-pin:** if URL capture yields nothing within the capture window (older CLI, e.g. ≤2.1.183 behavior), fall back to today's tier 2→3 order unchanged. No hard dependency on CLI version; 2.1.198 (pinned) is confirmed good.

### 3.3 The three surfaces

1. **New-agent launch** (`PreLaunchAuthPanel.tsx` / `launch-flow.ts`): replace "waiting for terminal login" with the in-app panel: URL shown + "Open browser" + paste box + live status. Existing `onTierChange`/phase plumbing already carries the state transitions.
2. **Credential-loss relogin** (agent pane, `useAgentControllerStatus.ts` path): when auth is lost mid-life, the pane's relogin affordance opens the same panel, targeting the agent's **existing** bound account dir (`existingAccountId` — refresh, don't mint; `run-provider-login.ts` already threads this).
3. **Armory + Agent Stash:**
   - `accounts-catalog.ts`: Anthropic entry gains `authModes: ["oauth", "key"]` (OAuth = this flow, not `oauth-catalog.ts`'s service-OAuth scaffold — that stays untouched).
   - Armory Accounts tile → Connect → in-app login panel → on success, account appears in the gallery (no agent link required; linking happens later at launch/Stash).
   - `AgentStashModal.tsx` Accounts tab (`AgentIdentityLinksPanel`): add "Connect / Re-login" action per binding, opening the same panel with `existingAccountId` + `linkTarget` set.
   - Requires decoupling `runProviderLogin` from agent-block context (today all 8 callers are pane/launch surfaces): the §3.1 session takes `{provider, accountId?, linkTarget?}` and no block id.

### 3.4 Out of scope (follow-ups)

- `setup-token` paste-a-token entry (headless/remote case, `REPORT_…RETHINK` §8's suggestion) — nice complement, separate PR.
- Codex/Gemini/OpenClaw parity — they already have working tier-1 URL capture; migrating them onto the §3.1 panel is mechanical but not this spec.
- Real service-OAuth (`identity/oauth_client.rs`) for Anthropic — still impossible (no device endpoint), still not needed.

## 4. Phasing

- **PR 1:** §3.1 core session + catalog changes (§3.2), wired into the launch surface (surface 1). Terminal tier kept as manual fallback.
- **PR 2:** relogin surface (surface 2) on the same primitive.
- **PR 3:** Armory + Stash (surface 3) incl. `accounts-catalog` change and the block-context decoupling.
- **PR 4 (cleanup):** retire auto-launch of tier 3; docs sweep (`catalog.ts` comment, `SPEC_HOST_CLI_LOGIN_CAPTURE` §0 note, `REPORT_…RETHINK` §8 addendum).

## 5. Security notes

- Credentials only ever land in the isolated per-account `CLAUDE_CONFIG_DIR` (invariant from #1626 preserved). Never log the authorize URL's full query (contains `state`/`code_challenge`), the pasted code, or minted tokens.
- The pasted code is single-use and PKCE-bound to the spawned process; a leaked URL alone is not a credential.
- Probe artifact: the 2026-08-03 verification minted one real 1-year token into a throwaway dir (deleted); it was displayed in a transcript and should be revoked from the account's Claude settings.
