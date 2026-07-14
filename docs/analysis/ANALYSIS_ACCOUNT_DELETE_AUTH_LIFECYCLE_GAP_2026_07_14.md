# Account deletion does not deauthenticate anything — auth lifecycle gap

**Date:** 2026-07-14
**Status:** Report — live-reproduced, not yet fixed
**Repro:** During auth stress-testing on a fresh `task dev` (v0.53.5, main
`1d5fa3f5`): user deleted the Anthropic account(s) in **Armory → Accounts**;
a running Claude agent **kept responding**, still fully authenticated.
**Related:** `SPEC_OAUTH_IDENTITY_BUNDLES_2026_05_22.md` (the injection
design), `SPEC_PROVIDER_ISOLATION_2026_06_20.md`, PR #2157 (logout-side
diagnostics that made this gap observable), PR #2158 (OAuth account UI crash
found the same session).

---

## 1. Expectation vs. reality

The natural user model: *deleting an account in the Armory revokes that
identity — agents using it stop being authenticated.* The actual behavior:
deletion is **pure bookkeeping**. `deleteidentityaccount`
(`agentmux-srv/src/server/agent_handlers/identity.rs:400-460`) does exactly
three things:

1. If the secret lives in the OS keychain (`SecretRef::Keychain`), delete it.
2. `DELETE FROM db_accounts WHERE id = ?1`
   (`backend/storage/identities.rs:261-265` — single-row delete, no cascade).
3. Publish `identityaccounts:changed` (a UI-refresh event, nothing more).

Nothing else in the system reacts. Four distinct layers keep the agent
authenticated:

## 2. The four layers of the gap

### 2.1 Running agent processes are untouched

No code path connects account deletion to live agents. The CLI process was
spawned with its credentials already injected (env vars for api-key class,
`CLAUDE_CONFIG_DIR` pointer for OAuth class, `identity/resolver.rs:600-650`)
and holds working tokens for its lifetime. There is no kill, no restart, no
notification, not even a UI chip on affected agent panes. **This alone fully
explains the repro** — the responding agent simply already had its creds.

### 2.2 OAuth tokens survive on disk (and are never revoked upstream)

For OAuth accounts the "secret" is `SecretRef::OAuthConfigDir { dir }` — a
pointer to a config dir (e.g. `~/.agentmux/shared/identities/<id>/claude/`)
holding the CLI's **live access + refresh tokens**. The delete handler's
keychain cleanup explicitly matches only `SecretRef::Keychain`; the
OAuthConfigDir variant falls through, so **the token directory survives the
account row**. Nothing calls the provider's revocation endpoint (or
`claude logout`) either — the tokens aren't just present, they're *valid*.
Anyone (or any later spawn) pointing at that dir is still authenticated.
Note the irony: the handler's own comment says the keychain cleanup exists
"so no orphaned credential survives the DB row" — for OAuth accounts an
orphaned credential is exactly what survives.

### 2.3 Next spawn silently falls back to ambient credentials

At spawn, per-binding resolution treats a missing account row as a
log-and-skip (`resolver.rs:577-590`: "account … bound to identity … but row
not found — skipping"). With no injection performed, the spawned Claude CLI
does its default thing: read `~/.claude` — the user's **global, ambient
login**. So even after a restart, the agent typically *still* authenticates,
now on a different (unmanaged, invisible-to-Armory) identity, with only a
WARN in the srv log to show for it. The "deleted the account" and "agent
still works" states are indistinguishable in the UI.

### 2.4 Orphaned junction rows

`identity_delete` doesn't cascade `db_agent_identity_links` rows that
reference the deleted `account_id`. (This report originally also named
`db_identity_bindings` — that table was retired in migration Phase 4c and no
longer exists; links are the sole junction on accounts, and a test now pins
that.) Orphans persist as dangling references, discovered only as per-spawn
WARNs (§2.3). Subtlety found during remediation: the *current* DDL does carry
`FOREIGN KEY … ON DELETE CASCADE` with `PRAGMA foreign_keys=ON`, but legacy
user databases got the links table via forge-era `ALTER TABLE … RENAME`,
which never retrofits FK clauses — so on real installs the DDL-level cascade
silently doesn't exist and the orphans are real (proven by a red test against
a legacy-shaped schema).

## 3. Severity

**High for the product's own promise.** The Armory is pitched as the control
surface for agent credentials; a delete that doesn't deauthenticate breaks
the core mental model, and the ambient-fallback layer (2.3) makes it worse
than a no-op — the agent silently migrates to the user's personal login.
For a machine shared or used for demos, "I deleted the account" ≠ "the agent
lost access" is a genuine security-expectation violation, though not an
external vulnerability (everything stays on the local machine, under the
same OS user).

## 4. Remediation options (roughly in dependency order)

1. **Cascade + cleanup at delete time** (smallest, clearly correct) —
   **implemented, PR #2159**:
   - Delete `db_agent_identity_links` rows referencing the account, in the
     same transaction as the account row (fixes 2.4).
   - For `OAuthConfigDir` accounts, delete the config dir (fixes 2.2's
     on-disk half). Best-effort provider-side revocation (running the CLI's
     own `logout` against that dir first) fixes the other half.
2. **Reconcile running agents**: on account delete, look up agents linked to
   it (the reverse index exists — `agentsAssignedToAccount`) and either
   (a) hard-stop their blocks, or (b) surface a "credentials revoked —
   restart required" state on the pane. (a) is the honest semantics;
   (b) is less disruptive but leaves 2.1 half-open until the process ends.
3. **Kill the silent ambient fallback**: when a binding exists but the
   account row is gone (or the binding was cascaded away in #1 and the agent
   has no account for an oauth-class provider), spawn should either fail
   loudly ("no credentials for provider anthropic") or require an explicit
   per-agent "use ambient/global login" opt-in. Silent fallback to `~/.claude`
   is the layer that turns every other fix into a no-op.
4. **UI truthfulness**: while any agent still holds credentials from a
   deleted account (2.1b chosen over 2.1a), the Armory should show it —
   e.g. a "revoked, N agents still hold tokens" row state instead of the row
   simply vanishing.

Option 3 is the load-bearing one: as long as ambient fallback exists, delete
semantics can never be made honest for oauth-class providers on a machine
where the user is also logged in globally.

## 5. Diagnostics note

This gap was only cleanly observable because of PR #2157 (logout-side
`identity.delete:`/`identity.unlink:` info logging + `muxlog auth`). The
delete itself now logs; what's missing is everything that *should* happen
after it. The resolver's "row not found — skipping" WARN at next spawn is
target `identity`, so it also surfaces in `muxlog auth` — the full lifecycle
of the bug is now traceable end to end:
`identity.delete: account removed` → (agent keeps responding, no log at all —
layer 2.1) → next spawn: `account … bound … but row not found — skipping` →
CLI silently uses `~/.claude` (no AgentMux log line at all — the fallback is
invisible even to the srv, which is itself part of the gap).
