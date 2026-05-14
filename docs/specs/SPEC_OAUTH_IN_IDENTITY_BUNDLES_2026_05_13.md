# Spec: OAuth credentials in identity bundles

**Status:** Spec (no implementation yet)
**Owner:** AgentA
**Date:** 2026-05-13
**Driving requirement:** "When I log into Claude via the OAuth browser flow once, the next time I launch an agent with the same Identity bundle I shouldn't have to log in again. The tokens should ride with the bundle."

---

## 1. TL;DR

Today an Identity bundle stores **references** to credentials (`SecretRef::Env`, `SecretRef::SecretsManager`, `SecretRef::PlaintextDev`). That works for long-lived secrets (PATs, API keys) where AgentMux just injects an env var at spawn time. It does **not** work for OAuth, which has:

- A pair of tokens that refresh on a schedule (access ~hours, refresh ~weeks)
- Periodic rotation that needs to be written back somewhere
- A native location the CLI expects them in (`<workdir>/.claude/credentials.json` for Claude Code)

Today the Claude OAuth flow writes into the agent's per-launch working dir. The next time that bundle launches anywhere else, no credentials → fresh OAuth → user has to authorize the same browser-popup again.

This spec adds a new `kind: "oauth"` account type backed by a per-bundle credentials file at `~/.agentmux/identities/<bundle_id>/<provider>/credentials.json`. On spawn the file gets symlinked or copied into the agent's working dir; on every write the host watches the source location and propagates back, so refresh rotations stay current. End user logs in once per bundle per provider.

Two PRs:

- **PR 1 — Plaintext-on-disk MVP.** Schema bump (v9), new `SecretRef::OAuthCredentialsFile`, spawn-time copy/symlink, notify-based capture loop. Credentials are not encrypted at rest; permissions on the file are 0600 (Unix) / ACL'd (Windows). PR-G's export/import still works — the file is just an extra payload.
- **PR 2 — Keyring-backed encryption.** Adds a per-machine master key in the OS keychain (`keyring` crate). Credentials are encrypted as a SQLite BLOB instead of a file on disk. Export to another machine re-encrypts under an export passphrase.

PR 1 ships the user-facing reuse story end-to-end. PR 2 hardens the at-rest story.

---

## 2. Today's state

### Identity bundle shape

```rust
pub struct IdentityAccount {
    pub id: String,
    pub name: String,
    pub provider: String, // "github" | "aws" | "anthropic" | "custom"
    pub kind: String,     // "pat" | "role" | "api_key" | "env_ref"
    pub display_name: String,
    pub secret_ref: SecretRef,
    pub context: serde_json::Value,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

pub enum SecretRef {
    Env { env_var: String },
    SecretsManager { sm_path: String, sm_json_path: Option<String> },
    PlaintextDev { plaintext_dev: String },
}
```

Resolution at spawn time (in `identity::resolver`) flattens this into `env_vars` injected into the agent CLI subprocess. For `Env`, the host's existing env passes through; for `SecretsManager`, the secrets-cli is shelled out; for `PlaintextDev`, the value is written directly.

### Claude Code OAuth flow today

1. User clicks **Launch Claude Code**. Host spawns `claude` with cwd = `<bundle's working dir>`.
2. Claude CLI looks for `<cwd>/.claude/credentials.json` (or `~/.claude/credentials.json` depending on version).
3. Not found → opens `https://console.anthropic.com/oauth/...` in the user's default browser.
4. User authorizes. Claude completes the PKCE handshake and writes the credentials file.
5. On every API call thereafter Claude refreshes its access token in-place when the cached one expires; the file is rewritten with the new token.

Once the working directory is reused (via named-agent continuation, PR #816) the credentials persist *for that working directory*. But launching a fresh agent on a different working dir with the same Identity bundle = new dir, no credentials, redo OAuth.

### The gap

The Identity bundle is the user's intent ("this is *AgentA-asaf's* GitHub + Claude logins, use these everywhere I bind this bundle"). The credentials file lives at the working-dir level, not the bundle level. Same logical identity, two different launch dirs → two OAuth flows.

---

## 3. Goals

1. **Single OAuth flow per (bundle, provider).** First launch in any working dir triggers OAuth and captures the result; every future launch using that bundle gets the credentials handed to it.
2. **Refresh rotations propagate.** When Claude refreshes its access token mid-session, the new token is captured into the bundle so the next launch starts with the latest.
3. **Survive working-dir creation/destruction.** The bundle's credentials live independently of any single agent's `~/.agentmux/agents/<name>-<suffix>/` dir.
4. **No regressions for non-OAuth kinds.** PAT, role, api_key, env_ref keep working as today.
5. **Coexist with the cross-version registry** (PR #819). Identity bundles are still per-version SQLite today; this spec doesn't change that. PR-G (export/import) is what makes bundles portable across versions.

### Non-goals

- **No cross-machine sync.** A machine's credentials stay on that machine unless explicitly exported (PR-G).
- **No bundling of CLI auth state beyond OAuth.** Cookies, browser sessions, model caches — out of scope.
- **No support for Claude API key + OAuth in the same account.** One account = one secret_ref variant.
- **No first-class UI for multiple OAuth accounts per provider** in this spec. The data model supports it; the picker can come later.

---

## 4. Schema (v9)

### New `SecretRef` variant

```rust
pub enum SecretRef {
    Env { env_var: String },
    SecretsManager { sm_path: String, sm_json_path: Option<String> },
    PlaintextDev { plaintext_dev: String },
    /// Path to a credentials file on the local machine that this bundle
    /// owns. PR 1: written + read as plaintext JSON (file-system perms
    /// gate access). PR 2: replaced by `OAuthBlob` with keyring-backed
    /// encryption.
    OAuthCredentialsFile {
        /// Path relative to `<shared_home>/identities/<bundle_id>/`,
        /// e.g. `"claude-code/credentials.json"`. Stored relative so
        /// PR-G export survives the home-dir change on import.
        path: String,
        /// Last successful refresh timestamp — diagnostic + helps the
        /// UI show "expires in 12 days" type warnings.
        last_refreshed_at_ms: i64,
    },
}
```

### New `kind` value

`IdentityAccount.kind = "oauth"`. The frontend's bundle editor adds a third row alongside "PAT" and "API key": "OAuth — log in to provider".

### Storage layout

```
~/.agentmux/
├── identities/
│   └── <bundle_id>/                       # mirrors db_identities row
│       └── <provider>/                    # e.g. claude-code, github
│           └── credentials.json           # the CLI's expected shape
└── agents/                                # working dirs, unchanged
```

The `identities/` tree sits next to `agents/`. It is **shared across versions** (same rationale as the named-agent registry). Each `<provider>` dir holds whatever file shape the provider's CLI expects — we don't reformat or transform, we just persist the file the CLI wrote.

### v9 migration

- Add columns to `db_identity_accounts`: none (the new variant is JSON-encoded inside `secret_ref`).
- Bump `schema_version` constant in `agentmux-srv/src/backend/storage/schema.rs`.
- No data migration — existing rows keep their `kind` and `secret_ref` shape.

---

## 5. Spawn-time injection

When `launch_forge_agent` resolves an Identity bundle with an `OAuth` account:

1. **Resolve the source path**: `<shared_home>/identities/<bundle_id>/<provider>/credentials.json`. If file doesn't exist yet (first-ever launch with this bundle), continue without it — the CLI will trigger OAuth and write the file.
2. **Resolve the target path** inside the agent's working dir:
   - `claude-code` → `<workdir>/.claude/credentials.json`
   - other providers → table in `agentmux-srv/src/identity/oauth_targets.rs`
3. **Materialize**:
   - **Linux/macOS**: symlink `<target>` → `<source>`. Single source of truth; writes by the CLI propagate immediately without a copy-back step.
   - **Windows**: hardlink when possible; fall back to copy + reverse-watcher (§6) when crossing volumes or filesystems that don't support hardlinks.
4. **Permissions**: `chmod 0600` on the source file (Unix); on Windows, set DACL to current user only.

### Edge cases

| Case | Behavior |
|---|---|
| Source missing | Continue without it. CLI does the OAuth flow itself; watcher (§6) captures the result. |
| Target already exists (e.g. user manually pasted creds) | Don't overwrite. Log a warning. User can clear and re-launch. |
| Symlink unsupported (Windows, no admin rights, fallback to hardlink failed) | Copy file; mark this account `materialized_via: "copy"` in memory; arm the reverse-watcher. |

---

## 6. Auto-capture watcher

Two cases that both feed the same path:

**Case A — first OAuth.** No source file existed at spawn. Claude does OAuth → writes `<target>`. Watcher sees the write on `<target>`, copies it to `<source>`.

**Case B — refresh rotation.** Source file existed; spawn copied (not symlinked) it to target. Claude refreshes → writes new content to `<target>`. Watcher copies to `<source>`.

Implementation lives in `agentmux-srv/src/identity/oauth_watcher.rs`. Hooks into the existing `notify` crate already in deps.

### Scope

Per-spawn watcher. Lifecycle tied to the agent process:

- **Arm** on spawn, scoped to `<target>` only (single-file watch).
- **Disarm** on agent exit (process termination handler).

Cross-spawn coordination not required at this layer — only one agent process owns `<target>` at a time (working dir is per-spawn).

### Idempotent writes

After every captured write we:
1. Read source + target byte-by-byte
2. Skip the write if identical (avoid feedback storms when Claude rewrites with the same content)
3. Update `last_refreshed_at_ms` in the bundle's `secret_ref`

### Failure modes

- **Watcher fails to arm**: log + degrade to "no auto-capture this session". User has to re-login on next spawn. Non-fatal.
- **Write to source fails (disk full, permissions)**: log + retry once after 500ms. If still failing, leave target alone (CLI keeps working for this session) and skip the bundle update.
- **Source disappears between watch arm and write**: re-create the directory, then write.

---

## 7. Refresh handling

Already handled in §6 — refresh is just another write to `<target>`. The watcher doesn't distinguish "first OAuth" from "refresh rotation"; both go through the same capture path.

### `last_refreshed_at_ms` UI

The bundle editor shows "Last refreshed: 2 minutes ago" alongside the OAuth account. Warning state when more than 80% of the average refresh interval has elapsed without a write — usually means the agent isn't being launched and the refresh token is approaching its end-of-life.

---

## 8. Concurrency

### Within a single machine

Multiple agents launching with the same Identity bundle SIMULTANEOUSLY:

- **Read path (spawn-time materialize)**: pure read of source file. Safe in parallel.
- **Write path (watcher capture)**: two agents both refresh → two writes to source. SQLite-style atomicity via `tempfile` + `rename`:
  1. Write new contents to `<source>.tmp.<pid>`
  2. `fsync`
  3. `rename` over `<source>` (atomic on every supported OS)
  4. Last write wins — both writes were valid refreshes from the same provider, so either is correct.

### Cross-machine

Out of scope. PR-G's export/import is single-direction (machine A → machine B); we don't try to merge concurrent refreshes from two machines.

---

## 9. PR 2 — Keyring-backed encryption

PR 1 stores credentials as a plaintext file at `~/.agentmux/identities/<bundle_id>/<provider>/credentials.json` with 0600 perms. That's the same threat model as the CLI's native storage (Claude itself stores plaintext in `~/.claude/credentials.json`). Good enough for v1.

PR 2 hardens this by encrypting at rest:

1. **Master key** in OS keychain via the `keyring` crate:
   - Linux: `secret-service` (gnome-keyring/kwallet)
   - macOS: Keychain Services
   - Windows: Credential Manager
2. **Per-account blob** stored as a BLOB column in `db_identity_accounts`, schema variant becomes `SecretRef::OAuthBlob { ciphertext: Vec<u8>, iv: Vec<u8>, last_refreshed_at_ms: i64 }`.
3. **Spawn-time decrypt → write to target** (existing flow), watcher captures + encrypts → upsert blob.
4. **Cipher**: AES-256-GCM. AEAD with the account id as associated data so a blob extracted from one row can't be replayed into another.

This is a separate PR because:
- It adds a workspace dep (`keyring`, `aes-gcm`)
- It needs platform-specific install/CI testing
- The user-facing reuse story works without it (PR 1 ships the win)

PR-G's export/import path becomes:
- Export: decrypt under the local master key → re-encrypt under a passphrase the user types → tarball
- Import: read tarball → decrypt under passphrase → re-encrypt under destination machine's master key

---

## 10. Frontend

Touching `frontend/app/view/agent/components/IdentityPaneAccountsTable.tsx` (or equivalent — check the current identity bundle editor):

### New row in the bundle's accounts table

```
┌─────────────────┬───────────────┬─────────────────────┬──────────────┐
│ Provider        │ Kind          │ Status              │ Action       │
├─────────────────┼───────────────┼─────────────────────┼──────────────┤
│ Claude Code     │ OAuth         │ ✓ Last refreshed    │ [ Re-login ] │
│                 │               │   12 minutes ago    │              │
├─────────────────┼───────────────┼─────────────────────┼──────────────┤
│ GitHub          │ PAT (existing)│ ✓                   │ [ Edit  ]    │
└─────────────────┴───────────────┴─────────────────────┴──────────────┘
```

### "Add OAuth account" flow

1. User picks **Add account** → **OAuth** → provider dropdown
2. Modal: "Click 'Authorize' to log into Claude Code"
3. Click → host spawns `claude` in a one-shot "auth only" mode against a scratch working dir → user does OAuth in browser → credentials file lands → host copies into `~/.agentmux/identities/<bundle_id>/<provider>/credentials.json`
4. Modal closes; bundle now has the OAuth account

The "scratch working dir" can be `~/.agentmux/identities/<bundle_id>/<provider>/auth-tmp/` — deleted after capture.

### "Re-login" affordance

Same flow as add, except the source path stays the same — overwritten with the new credentials file.

---

## 11. Test plan

### Unit (Rust)

- **`SecretRef` round-trips through serde** for the new variant.
- **Spawn-time materializer** with mock filesystem:
  - Symlink path on Unix, hardlink fallback on Windows
  - Permissions set correctly
  - Missing source = continue silently
  - Existing target = log + skip (no overwrite)
- **Watcher**:
  - Captures first OAuth write
  - Captures refresh rotation
  - Idempotent on identical-content writes
  - Survives source-directory disappearance + recreation

### Integration (srv test harness)

- **End-to-end first OAuth**: spawn agent with bundle that has empty `OAuthCredentialsFile`, mock the Claude CLI writing a credentials file at `<workdir>/.claude/credentials.json`, verify `<source>` exists + content matches.
- **End-to-end refresh**: pre-seed `<source>`, spawn agent (materialize), mock CLI writing updated content to `<target>`, verify `<source>` updated.
- **Concurrent refresh**: two test threads both refresh; assert final `<source>` is one of the two write contents (last-write-wins).

### Frontend (vitest)

- Bundle editor renders OAuth row when `kind == "oauth"`.
- "Re-login" action POSTs to the right RPC.
- Status string formats `last_refreshed_at_ms` correctly across time ranges (seconds / minutes / hours / days / weeks).

### Manual smoke

- Empty bundle → click "Launch Claude Code" → OAuth in browser → credentials captured. Verify `~/.agentmux/identities/<bundle_id>/claude-code/credentials.json` exists.
- Kill the agent, launch a fresh one with the same bundle → no OAuth prompt, agent ready immediately.
- Let the agent run for >1 hour to trigger a refresh → verify `last_refreshed_at_ms` updates in the UI.

---

## 12. PR sequence

### PR 1 — Plaintext-on-disk OAuth (this spec's MVP)

- Schema v9 migration (constant bump, no data move)
- New `SecretRef::OAuthCredentialsFile` variant
- `agentmux-srv/src/identity/oauth_targets.rs` — provider → target-path table
- `agentmux-srv/src/identity/oauth_materialize.rs` — spawn-time copy/symlink
- `agentmux-srv/src/identity/oauth_watcher.rs` — capture loop
- Frontend bundle editor: OAuth row + "Add OAuth account" + "Re-login" action
- Tests per §11
- Bump patch + smoke

### PR 2 — Keyring-backed encryption (follow-up)

- Add `keyring`, `aes-gcm` deps
- New `SecretRef::OAuthBlob` variant
- Migration from `OAuthCredentialsFile` → `OAuthBlob` (read file, encrypt, store BLOB, delete file)
- Export/import passphrase wrapping (folds into PR-G)
- CI matrix: Linux gnome-keyring, macOS Keychain, Windows Credential Manager
- Tests + smoke per platform

PR 2 depends on PR 1 merging first.

---

## 13. Risks + mitigations

| Risk | Mitigation |
|---|---|
| Symlink across Windows volumes fails | Hardlink fallback; if both fail, copy + reverse-watcher (§6 already covers this) |
| File watcher misses a refresh (notify crate inotify saturation, missed FS event) | Reconcile on agent exit: compare source vs target one final time and copy if different |
| User logs into two different Anthropic accounts using the same bundle | Detected at OAuth completion — Anthropic's `account_id` in the credentials file differs from what we cached. Surface as a confirm modal: "Replace stored login?" |
| Plaintext on disk leaks via backup software | PR 2 (encryption); document the threat model in PR 1's release notes |
| Refresh token expires while no agent is running | Show the bundle as `Status: requires re-login` if `last_refreshed_at_ms` exceeds provider's known refresh-token TTL. Re-login is one click. |
| Cross-version SQLite (PR #819 scope): identity bundles still per-version | Out of scope — the registry only covers named agents. Bundles are still cleaned up when a version is uninstalled. Cross-version bundle store is its own future spec. |

---

## 14. Open questions

1. Should we **support multiple OAuth accounts per provider** in a single bundle (e.g. work + personal Anthropic logins)? The data model supports it (multiple `IdentityAccount` rows with same provider, different kinds). UI gets more complex.
2. Should the **scratch working dir for "Add account"** be visible to the user (e.g. in their file manager), or hidden under `~/.agentmux/`? Hidden is simpler; visible is more transparent.
3. **Auto-detect existing `~/.claude/credentials.json`** at bundle creation time and offer to import it? Saves users from re-OAuthing if they already have a system-wide login. Tradeoff: opens an attack surface where a malicious bundle could siphon a user's existing auth.
