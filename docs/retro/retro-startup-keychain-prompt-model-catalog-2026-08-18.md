# Retro: opening AgentMux on macOS prompted for the Keychain password

**Date:** 2026-08-18
**Area:** `agentmux-srv/src/backend/model_catalog.rs`, `agentmux-srv/src/server/providers_handlers.rs`, `frontend/app-init.ts`

---

## 1. Symptom (as reported)

A freshly built v0.55.13 `.dmg`, opened for the first time: macOS immediately
prompted for the Keychain password. Reported as unacceptable — "we cannot
have that prompt."

## 2. Investigation

- Grepped for AgentMux's own direct OS-Keychain usage (separate from the
  Claude Code CLI's own Keychain use, which is the subject of
  [retro-macos-keychain-credential-isolation-gap-2026-08-17.md](retro-macos-keychain-credential-isolation-gap-2026-08-17.md)
  and out of AgentMux's control). Found `identity/secret_store.rs`, a thin
  wrapper around the `keyring` crate under a fixed service name (`"agentmux"`)
  — this is a real, first-party Keychain touch, not the Claude CLI's.
- Traced every caller of that module. Most are gated behind explicit user
  actions (connecting/deleting an Armory account, MuxBus token storage). One
  wasn't: `backend/model_catalog.rs`'s `resolve_access_token`, used by the
  `providers.models` RPC (`server/providers_handlers.rs`) — a "fetch the
  authoritative Claude model list for the dropdown" feature.
- `resolve_access_token`'s own doc comment already flagged the risk before
  this investigation: "can even trigger a macOS permission prompt on first
  access" — the mechanism was known; what wasn't caught was where it's
  called from.
- `frontend/app-init.ts:1108` calls `providers.models` **fire-and-forget, on
  every app launch**, unconditionally, as part of the main init wave — not
  gated behind opening the model dropdown or any other user action.

## 3. Root cause

`resolve_access_token` tries three sources in order: (1) the isolated
`.credentials.json` file (Linux/Windows — no Keychain involved), (2) the
`CLAUDE_CODE_OAUTH_TOKEN` env var (if set, persisted into AgentMux's own
Keychain entry for reuse across relaunches), (3) that same persisted
Keychain entry, read back unconditionally whenever step 2's env var isn't
present.

On macOS, step 1 always fails — Claude Code never writes `.credentials.json`
there (the same root cause as the 2026-08-17 retro). So on every macOS
launch, this automatic background call falls straight to step 3: an
unconditional read of AgentMux's own `"agentmux"` Keychain entry. If that
entry already contains something (e.g. from an earlier session where
`CLAUDE_CODE_OAUTH_TOKEN` was exported once), macOS's Keychain ACL requires
approval from any code signature it doesn't already trust — which, for a
locally rebuilt dev binary, is every fresh build. The result: a password
prompt at app open, for a purely cosmetic feature (fresher model-name
labels in a dropdown) the user never asked to trigger.

## 4. Fix

Added `allow_keychain_fallback: bool` to `resolve_access_token`. The
automatic startup RPC (`providers.models`) now passes `false`, skipping
steps 2/3 (and the `spawn_blocking` they ran on) entirely — no Keychain
interaction from that path, ever. The underlying capability isn't deleted
(steps 2/3 still exist in the function, callable with `true`), since it's a
real, documented, intentional feature for users who explicitly export
`CLAUDE_CODE_OAUTH_TOKEN` — it's just no longer reachable from a silent,
unprompted background call. No caller passes `true` today; that's left for
a future explicit-user-action trigger if one is ever built, not implemented
speculatively here.

## 5. Why this wasn't caught earlier

The risk was documented in the function's own doc comment when the
Keychain-fallback feature was added, but the audit stopped at "this can
prompt" without asking "is this call site the kind where a prompt is
acceptable?" The feature's own doc comments correctly frame it as
best-effort/cosmetic ("Best-effort — returns \[\] with no token"), which
made it easy to reason about failure modes (falls back to static list) while
missing the different question of side effects on success — the OS-level
interaction has a UX cost even when the read eventually succeeds.

## 6. Follow-up

None planned. If a future explicit "refresh model catalog" user action is
ever built, it can pass `allow_keychain_fallback: true` — do not wire the
automatic/background path back to it without re-reading this retro.
