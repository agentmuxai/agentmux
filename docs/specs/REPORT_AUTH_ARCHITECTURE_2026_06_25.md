# Auth Architecture Report — AgentMux

**Date:** 2026-06-25
**Purpose:** Design-session input for making auth robust app-wide. Covers current state, the terminal-vs-agent gap, the "Login Again" crash, and gaps to fix.

> **Staleness note (2026-08-03):** table/column names here (e.g. `db_identity_accounts`) predate the 2026-07-12 Phase 4a rename to `db_accounts`. See `docs/specs/REPORT_AUTH_ARCHITECTURE_STATE_AND_RETHINK_2026_07_21.md` for a later pass over the same territory (itself also flagged as partially stale — check both against current code before relying on either for schema names).

---

## 1. How Credentials Are Stored

Three independent storage layers co-exist today:

### Provider OAuth tokens (per-bundle, on disk)

- **Global (terminal ambient):** `~/.claude/.credentials.json` — what the terminal pane reads
- **Isolated (per agent-identity bundle):** `~/.agentmux/identities/<bundle-id>/claude/.credentials.json`
- Set via `CLAUDE_CONFIG_DIR` env var at spawn time
- Same pattern for Codex (`CODEX_HOME`) and OpenClaw (`OPENCLAW_HOME`)

### Identity account DB

`agentmux-srv/src/backend/storage/identities.rs`
- `db_identity_accounts` — each row: `provider`, `kind` ("oauth" | "api-key"), `secret_ref`, `status` ("valid" | "expired")
- `db_identity_bindings` — joins `(bundle_id, provider) → account_id`
- `db_identity_bundles` — named credential sets the user creates (e.g. "personal", "work")

### MuxBus cloud credentials (singleton)

`agentmux-srv/src/backend/storage/muxbus.rs`
- One global row in `db_muxbus_credentials`: `access_token`, `refresh_token`, `user_email`, `cognito_domain`, `expires_at`
- Not per-agent; injected into every agent env if valid

---

## 2. The Terminal-vs-Agent Auth Gap

### Why terminal pane is authed

The terminal pane spawns a shell with **no env override** for `CLAUDE_CONFIG_DIR`. The shell inherits whatever the user set up in `~/.claude/` — typically a valid credential from an earlier `claude setup-token` or OAuth login.

### Why agent pane shows "Login again"

At agent spawn, `inject_identity_env_async()` (`agentmux-srv/src/identity/resolver.rs:369–410`) always sets:

```
CLAUDE_CONFIG_DIR = ~/.agentmux/identities/<bundle-id>/claude/
```

If that directory has no `.credentials.json`, Claude CLI sees an empty credential store and reports `authenticated: false`. The frontend sees this and shows the "Login again" banner.

**No credential copy happens by default.** The global `~/.claude/.credentials.json` is never automatically seeded into the bundle-specific dir. The user sees 401s even though their terminal is fully logged in.

The disconnect between "my terminal works" and "my agent says login again" is the core UX problem.

---

## 3. The "Login Again" Button Does Nothing — Root Cause

### Call chain

```
User clicks "Login Again"
  → useAgentControllerStatus.ts:171 → forceProviderLogin()
  → force-login.ts:43 → getApi().runCliLogin(cliPath, authLoginArgs, authEnv, requiresTty)
  → CEF IPC: run_cli_login_pty (agentmux-cef/src/commands/platform.rs:656)
  → portable_pty::CommandBuilder::spawn()
  → claude auth login (Bun-compiled)
  → CRASH: "FD ownership violation" before any output
  → CEF sees child exit, no auth URL captured, nothing returned to frontend
```

### Why it crashes

The Bun runtime enforces strict FD ownership at startup. When CEF's `portable_pty` spawns the Claude CLI, the ConPTY handle lifetime is violated — the `pair.master` is dropped too early, leaving the child process with dangling handles. Bun detects this and aborts before printing anything.

Evidence: `SPEC_HOST_CLI_LOGIN_CAPTURE_2026_06_20.md` §0–2 confirms zero `[login-pty]` output lines from the CEF spawn path. The backend's `auth.start()` PTY path (identity_handlers.rs) has the same underlying bug but was masked by earlier versions.

### What "Use Existing Login" does (the working workaround)

`useAgentControllerStatus.ts:212–236` offers a second button that calls `seed_provider_auth_from_global()` on the host — this copies `~/.claude/.credentials.json` into the bundle-specific dir without spawning the Bun CLI at all. This works reliably. The problem is it's hidden behind a secondary button and users never find it.

---

## 4. Recent Significant Auth Work

| PR / Commit | What |
|---|---|
| #1775 (`19178949`) | Deduplicate identity accounts that share the same credential |
| `d6341531` / `1752f5fa` | MuxBus cloud login chip in statusbar HostPopover |
| `SPEC_HOST_CLI_LOGIN_CAPTURE` (Jun 20) | Investigated the Claude v2.1.x login capture failure; confirmed FD crash; added instrumentation |
| `retro-claude-v2-1-auth-spawn-2026-06-23.md` | Post-mortem: headless CEF can't open browser for `setup-token` even if PTY works |
| `SPEC_AUTH_CHECK_FALSE_POSITIVE_2026_04_15.md` | Token presence ≠ validity — the fast-path `CheckCliAuth` could return `authenticated: true` on an expired token |
| `SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14.md` | OAuth session lifecycle design (`auth.start` / `auth.poll` / `auth.cancel`) |

---

## 5. Frontend Auth State — No Central Reducer

Auth state is **not centralized**. Each surface owns its own state:

| Surface | State hook | What it holds |
|---|---|---|
| Agent pane | `useAgentControllerStatus.ts` | `authUrl`, `canRetry`, `loginWaiting`, `agentReady` |
| Agent failure banner | `useAgentFailure.ts` | failure type, retry countdown, expanded |
| Armory / HostPopover | `useMuxBusStatus()` | `{connected, email, expiresAt, valid}` — re-fetched on open |
| Terminal pane | nothing | inherits ambient, never checks |

**Problem:** Cross-pane auth events are invisible. If the user logs in via the Armory, agent panes don't re-check. If a MuxBus token expires, nothing tells the agent panes proactively.

---

## 6. MuxBus Cloud Login Flow (Works Today)

1. User clicks "Connect" in HostPopover → frontend calls `muxbus.login(cognitoDomain, clientId)`
2. Backend opens browser to Cognito PKCE URL, blocks up to 5 min for callback
3. On callback, exchanges code → `access_token` + `refresh_token`; writes to `db_muxbus_credentials`
4. Every subsequent agent spawn: `inject_muxbus_env()` (agent_handlers.rs:3156) injects `MUXBUS_TOKEN` if `current_time < expires_at`
5. **No background refresh** — when the token expires, agents silently lose cloud auth until user manually reconnects

---

## 7. Auth Robustness Gaps (Priority Ordered)

### P0 — "Login Again" crashes silently

**Impact:** Auth recovery is broken for every user who hasn't discovered "Use Existing Login".
**Fix:** Either:
- Fix `portable_pty` usage in CEF — keep `pair.master` alive past `child.wait()` (matches how `agentmux-srv`'s auth.start already does it)
- OR bypass PTY entirely for Claude CLI login: use pipes + `--print-default-system-prompt` style callback URL extraction
- Short-term: **make "Use Existing Login" the primary button** and hide "Login Again" behind an "Advanced" expander

### P1 — No credential seeding on agent launch

**Impact:** Every agent pane launch requires manual intervention for new bundles.
**Fix:** On first agent spawn against a bundle with no credentials, proactively check if a valid global `~/.claude/.credentials.json` exists. If yes, auto-seed it (same logic as "Use Existing Login"). Show a toast: "Using your global Claude login — manage in Armory."

### P1 — No central auth reducer / cross-pane sync

**Impact:** Armory login doesn't update agent pane banners; MuxBus token expiry is silent.
**Fix:** Introduce a single SolidJS store atom (or Zustand-style slice) for app-wide auth state:
```ts
interface AppAuthState {
  muxbus: { connected: boolean; email: string | null; expiresAt: number | null }
  providers: Record<ProviderId, { authenticated: boolean; email: string | null }>
  lastChecked: number
}
```
Backend publishes a new WPS event `auth:changed` whenever any credential changes (login, logout, expiry). Frontend's central store subscribes and invalidates all panes.

### P2 — MuxBus token no background refresh

**Impact:** Agents silently lose cloud auth mid-session when the token expires.
**Fix:** Backend timer job that calls Cognito token refresh endpoint before `expires_at`. Already have `refresh_token` in `db_muxbus_credentials`. Emit `auth:changed` event after refresh.

### P2 — CheckCliAuth false positive

**Impact:** Agent spawns, "seems authed", 401s on first real API call.
**Fix:** `SPEC_AUTH_CHECK_FALSE_POSITIVE_2026_04_15.md` already specifies the fix — run `claude auth status --json` (slow path) rather than just checking file presence. Gate agent spawn on the slow-path result, with a spinner.

### P3 — Terminal-vs-Agent mental model mismatch

**Impact:** Confusing UX. Users think being logged in to the terminal means they're logged into the agent.
**Fix:** Surface the distinction early: on first agent pane open, show a one-time "Your global Claude login is available — using it for this agent" message (if seeding) or "Agents use isolated logins — sign in below" (if no seed available).

---

## 8. Proposed Architecture — Robust App-Wide Auth

```
Backend                                          Frontend
───────────────────────────────────────          ─────────────────────────────────
AuthStateService (new)                           useAppAuth() store (new SolidJS atom)
├─ watches db_muxbus_credentials                 ├─ subscribes to WPS auth:changed
├─ watches db_identity_accounts                  ├─ exposes { muxbus, providers }
├─ refreshes MuxBus token proactively            └─ invalidated on any auth event
└─ emits WPS auth:changed on any change
                                                 All panes read from useAppAuth():
AgentSpawnService (extend)                       ├─ Agent pane 401 banner
├─ pre-flight: seed global creds if bundle empty │  (no more per-pane polling)
├─ post-launch: subscribe agent to auth:changed  ├─ HostPopover MuxBus chip
└─ on auth:changed: update live agent env vars   └─ Armory panel

CEF CliLogin (fix)
├─ keep pair.master alive past child.wait()
├─ pipes path as fallback if PTY fails
└─ timeout + clean error propagated to frontend
```

---

## 9. Key File Reference

| File | What |
|---|---|
| `agentmux-srv/src/identity/resolver.rs:369` | `inject_identity_env_async` — sets `CLAUDE_CONFIG_DIR` per bundle |
| `agentmux-srv/src/server/agent_handlers.rs:3148` | Agent spawn env assembly |
| `agentmux-srv/src/server/muxbus_handlers.rs:50` | `muxbus.login` / `inject_muxbus_env` |
| `agentmux-srv/src/muxbus/pkce.rs:26` | Cognito PKCE browser flow |
| `agentmux-cef/src/commands/platform.rs:656` | `run_cli_login_pty` — the crashing PTY path |
| `frontend/app/view/agent/hooks/useAgentControllerStatus.ts:171` | "Login Again" button handler |
| `frontend/app/view/agent/hooks/useAgentControllerStatus.ts:212` | "Use Existing Login" (the working path) |
| `frontend/app/view/accounts/AgentMuxConnectPanel.tsx:59` | `useMuxBusStatus` hook |
| `docs/specs/SPEC_HOST_CLI_LOGIN_CAPTURE_2026_06_20.md` | FD crash investigation |
| `specs/SPEC_AUTH_CHECK_FALSE_POSITIVE_2026_04_15.md` | Token presence vs validity bug |
| `docs/specs/SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14.md` | OAuth session lifecycle design |
