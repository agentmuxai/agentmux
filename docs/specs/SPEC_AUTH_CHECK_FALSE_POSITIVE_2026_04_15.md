# SPEC: Auth Check False Positive — "authenticated as max" on Load

**Date:** 2026-04-15  
**Status:** Ready for implementation  
**Priority:** High — user must manually /login every session despite being told they're authenticated

---

## 1. Problem

When AgentMux loads an agent pane, the launch flow logs:

```
[auth] authenticated as max (claude.ai oauth)
```

…and skips the login flow entirely. But the agent is not actually authenticated — any real API call fails, and the user must run `/login` manually to recover.

---

## 2. Root Cause (two separate bugs)

### Bug A — Fast-path reads the credentials file but never validates the token

`cli_handlers.rs` `CheckCliAuth` has a fast path for Claude (`cmd.cli_path.contains("claude")`). It reads `~/.claude/.credentials.json` directly and checks:

```rust
let authenticated = has_token || has_refresh;
```

A non-empty `accessToken` or `refreshToken` string → `authenticated = true`. The tokens are **never validated against the Anthropic API**. Common failure modes:

| Scenario | File state | Fast-path result | Reality |
|----------|-----------|-----------------|---------|
| Session expired | old accessToken + refreshToken exist | ✓ authenticated | ✗ tokens rejected |
| Credentials revoked (logout on another device) | tokens still on disk | ✓ authenticated | ✗ revoked |
| Different account logged in globally | stale tokens from prior user | ✓ authenticated | ✗ wrong account |
| Isolated auth dir not yet populated | falls back to global `~/.claude/` | ✓ authenticated | ✗ wrong dir used |

### Bug B — `email` field is set to `subscriptionType`, not the user's email

```rust
email: subscription.clone(), // no email in creds, show subscription
```

`subscriptionType` in `.credentials.json` is a plan tier string (`"max"`, `"pro"`, `"free"`). This is emitted in the log as:

```
authenticated as max (claude.ai oauth)
```

…making the user think "max" is their username. The actual user email is NOT in `.credentials.json` — it's only available from `claude auth status --json`. Using the subscription type as a display name for the email field is misleading and the root cause of the confusing log message.

---

## 3. Correct Behavior

1. The launch flow should only skip `/login` if authentication is **actually working** — meaning the CLI confirms it can make API calls.
2. The log message should show the user's real email or a neutral "authenticated (max plan)" — never a plan tier in place of a name.
3. If token validation fails, the flow should proceed to the login phase, not silently proceed as if authenticated.

---

## 4. Proposed Fix

### 4.1 Drop the fast-path file read; use the CLI for Claude too

Remove the special-case `if cmd.cli_path.contains("claude")` block in `cli_handlers.rs`. Run `claude auth status --json` the same way other providers run their check command. This:

- Validates the token is actually accepted by Anthropic
- Returns the real `email` field from the JSON output
- Handles token refresh transparently (the CLI does it)
- Eliminates the file-path fallback ambiguity

Cost: ~1–2 s on first load instead of ~0 ms. Acceptable — the user is already waiting on the controller registration.

### 4.2 Parse email correctly from `claude auth status --json`

The `claude auth status --json` output includes:

```json
{
  "loggedIn": true,
  "emailAddress": "user@example.com",
  "subscriptionType": "max",
  "authMethod": "OAuth",
  ...
}
```

The slow-path parser already reads `json.get("email")` — but Claude's output uses `emailAddress`. Add a fallback:

```rust
email = json.get("emailAddress")
    .or_else(|| json.get("email"))
    .and_then(|v| v.as_str())
    .map(|s| s.to_string());
```

### 4.3 Display subscription type separately (optional UI polish)

In `launch-flow.ts`, the `auth_method` field already carries `"claude.ai oauth"`. Subscription can be forwarded in `raw_output` or a new `subscription` field on `CheckCliAuthResult` and logged as a secondary detail:

```
[auth] authenticated as user@example.com  (max plan · claude.ai oauth)
```

---

## 5. Files to Change

| File | Change |
|------|--------|
| `agentmux-srv/src/server/cli_handlers.rs` | Remove fast-path file-read block for Claude; let it fall through to the slow-path CLI runner |
| `agentmux-srv/src/backend/rpc_types.rs` | Optionally add `subscription: Option<String>` to `CheckCliAuthResult` |
| `agentmux-srv/src/server/cli_handlers.rs` | Add `emailAddress` → `email` fallback in slow-path JSON parser |
| `frontend/app/view/agent/flows/launch-flow.ts` | Update log message to include subscription if present |

---

## 6. Migration / Rollout

- No config changes required.
- The fast-path removal adds ~1–2 s to agent pane cold-start for Claude users. This is the correct trade-off — a 2 s delay is better than silently proceeding with invalid auth.
- If the slow-path times out (CLI unreachable), the existing `catch` block already sets `needsLogin = true`, which is the safe default.

---

## 7. Out of Scope

- Token refresh retry logic (the CLI handles this internally).
- Persistent session caching beyond what the CLI already does.
- Other providers (Codex, Gemini) — they already use the slow path.
