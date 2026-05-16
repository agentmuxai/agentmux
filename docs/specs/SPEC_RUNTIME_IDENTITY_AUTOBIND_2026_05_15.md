# SPEC: Runtime Identity Auto-Bind & Named-Agent Display Fix

**Status:** Draft / proposed
**Date:** 2026-05-15
**Author:** AgentA
**Related:** `SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14.md` (PR #850), `SPEC_AGENT_PANE_STATE_PERSISTENCE_2026_05_15.md` (PR #877)

---

## 1. Problem

Two related issues hit the same user flow:

### 1.1 Bug A — Display

The "Continue an existing agent" dropdown in the launch modal renders rows like:

```
Maks ·  ·  · 5m ago
```

with a visibly empty identity and memory column. Root cause:

- The named-agent record has `identity_id = ""` and `memory_id = ""` (the user didn't pick either at launch).
- The backend handler at `agentmux-srv/src/server/forge_handlers.rs:1208-1225` *does* substitute `"(ambient creds)"` and `"(vanilla CLI)"` for empty IDs.
- **The deployed `agentmux-srv-0.33.900` binary doesn't contain those strings.** `grep -c "ambient creds"` against `runtime/agentmux-srv-0.33.900-windows.x64.exe` returns 0. Cargo's incremental cache kept a pre-#816 object file even after `cargo clean -p agentmux-srv`. The source has the fix; the running binary doesn't.

So the user sees raw empty `identity_name` / `memory_name` strings rendered between the `·` separators.

### 1.2 Bug B — Runtime identity not captured

User flow on first Claude launch:

1. Open the launch modal. No Claude Identity exists yet → identity picker only offers "— Blank (no creds) —".
2. User proceeds, creates named agent "Maks" with `identity_id = ""`.
3. Pane opens. Claude CLI subprocess prompts for OAuth login.
4. User completes OAuth.
5. Claude CLI writes credentials to `~/.claude/credentials.json` (its own private store).
6. AgentMux's `db_identities` table stays empty. The "Maks" instance record's `identity_id` stays empty.
7. Forever after, "Maks" is rendered as `(ambient creds)` in the dropdown — even though Claude credentials actually exist on disk and the CLI is using them.

The runtime authentication is invisible to AgentMux's identity layer. Nothing bridges:

- The CLI's own OAuth-success event (which `agentmux-srv/src/identity/auth_session.rs` *can* detect from stdout, but only during the pre-launch flow).
- The `db_identities` table.
- The `AgentInstance.identity_id` FK on the already-created named-agent row.

## 2. Goals

- **G1** A named agent created without a pre-launch identity, then logged in during the runtime, displays as bound to an Identity record after the next reopen.
- **G2** The Identity record's name is sensible (e.g. `"Claude — user@example.com"` for a Claude OAuth login).
- **G3** No regression for users who *did* pick an identity at launch.
- **G4** Bug A (the visible blanks) is fixed regardless of whether G1–G3 ship — a defensive frontend fallback handles the empty-string case even if the backend sentinel substitution doesn't fire.
- **G5** Identity / credential layer stays the source of truth for which creds get injected at agent spawn (no breaking changes to `agentmux-srv/src/identity/resolver.rs`).

## 3. Non-goals

- Migrating existing `~/.claude/credentials.json` to AgentMux's `db_identity_accounts` (creds keep living where Claude wrote them; AgentMux just records a *reference*).
- Multi-account identities — v1 binds one credential to one Identity. Mixing GitHub PAT + Claude OAuth into the same bundle stays manual.
- Cross-provider auto-detect (Codex, Aider, etc.) in v1. Spec is structured so Phase 4 can extend.

## 4. Current architecture (what the audit revealed)

### 4.1 Data model

| Table | Key fields | Notes |
|---|---|---|
| `db_identities` | `id`, `name`, `is_blank`, `created_at` | Bundle row. `is_blank=1` is the seeded "— Blank (no creds) —" singleton. |
| `db_identity_accounts` | `id`, `name`, `provider`, `kind`, `secret_ref`, `status` | Individual creds. `secret_ref` is JSON: `Env`, `SecretsManager`, or `PlaintextDev`. |
| `db_identity_bindings` | `identity_id`, `provider`, `account_id` | Junction. One identity can bundle one account per provider. |
| `db_agent_instances` | `id`, `identity_id`, `memory_id`, `instance_name`, ... | FKs to the bundles. Empty string = ambient/blank. |

### 4.2 Resolver (env-var injection at spawn time)

`agentmux-srv/src/identity/resolver.rs:115` `inject_identity_env`:

1. Look up the instance for `block_id`.
2. If `identity_id == ""` or `"blank"` → return early. Subprocess inherits ambient credentials (Claude reads `~/.claude/`, etc.).
3. Else → walk `bundle_identity_bindings`, resolve each `secret_ref`, inject the provider's env-var matrix.

Failures are warn-don't-block — missing accounts / unresolved secrets are logged, the agent launches with whatever resolved cleanly.

### 4.3 Auth flow

`agentmux-srv/src/identity/auth_session.rs`:

- `AuthSessionStatus` state machine: `Pending` → `UrlAvailable` → `CodeEmitted` → `Success { bundle_id, email } | Failed`.
- `record_line(session_id, line)` parses CLI stdout via `auth_patterns.rs` and advances state.
- `finish_success(session_id, bundle_id)` transitions to `Success` once the handler confirms via `authCheckCommand`.

Pre-launch flow (PR #850, `PreLaunchAuthPanel.tsx`) wires this up: starts a session, polls state, renders OAuth URL, and *creates an Identity bundle* via the auth handler when the CLI signals success.

**Crucially**: that pre-launch wiring is only active when the user is in the pre-launch flow. Once the agent has launched without an identity, no AuthSession is associated with the running pane — the Claude CLI's later OAuth completion isn't observed by AgentMux.

### 4.4 Instance mutation

`wstore::instance_update` at `wstore.rs:1795` updates `block_id`, `session_id`, `status`, `github_context`, `ended_at`. **It does NOT touch `identity_id` or `memory_id`** — they're set at insert and never modified.

That's deliberate — historically nothing wanted to mutate them. Bug B is the first feature that needs to.

## 5. Proposed design

### 5.1 Phase 1 — Fix Bug A (display)

Two changes:

**Backend (one-time fix):** Rebuild `agentmux-srv` with `cargo clean` to invalidate the stale object cache. The source already substitutes the sentinels — the deployed binary just needs to actually contain them. Documented in §9.1.

**Frontend defensive fallback** at `frontend/app/view/agent/components/AgentLaunchModal.tsx:270`:

```ts
const parts = [
    row.instance_name,
    row.identity_name?.trim() || "(ambient creds)",
    row.memory_name?.trim() || "(vanilla CLI)",
];
if (row.started_at) parts.push(formatRelative(row.started_at));
return parts.join(" · ");
```

Even if a future backend regression returns empty strings, the user sees the labelled fallback instead of bare separators. Doesn't break the working case — if the backend already sent the sentinel, it stays.

### 5.2 Phase 2 — Runtime auth session reuse

Extend the existing `AuthSession` lifecycle so that **the launching agent pane creates and owns a session for the duration of any CLI-initiated OAuth flow**, not just for pre-launch.

#### Trigger

In `agentmux-srv/src/identity/auth_session.rs`, add an entry point invoked by the agent runner whenever a subprocess starts a fresh login:

```rust
pub fn start_runtime_session(
    provider_id: &str,
    instance_id: &str,        // NEW: which named-agent triggered this
    block_id: &str,           // NEW: which pane subprocess emitted it
) -> SessionId;
```

The runner (`agentmux-srv/src/agents/runner.rs`) feeds stdout lines into `record_line` exactly the way the pre-launch flow does. When `AuthPatternMatch::LoginSuccess { email }` fires:

1. Auth handler calls `authCheckCommand` to verify (same path as pre-launch).
2. On success, handler calls a **new** `finalize_runtime_session(session_id)` which:
   - Looks up an Identity matching `(provider, email)`; if none exists, **creates one** via `bundle_identity_upsert` (`wstore.rs:2013`) with name `"<provider> — <email>"` (e.g. `"Claude — user@example.com"`).
   - Records the cred reference as an `IdentityAccount` with `secret_ref = SecretRef::AmbientCli { provider }` — a **new variant** that signals "credentials managed by the CLI itself; we don't read them, just know they exist."
   - Inserts the binding via `bundle_identity_bind`.
   - **Updates the instance's `identity_id`** via a new `instance_update_identity(instance_id, identity_id, memory_id)` wstore method.

#### New `secret_ref` variant

```rust
pub enum SecretRef {
    Env { env_var: String },
    SecretsManager { sm_path: String, sm_json_path: String },
    PlaintextDev { plaintext_dev: String },
    AmbientCli { provider: String },   // NEW
}
```

The resolver (`resolver.rs:115`) treats `AmbientCli` as a **no-op** — don't inject anything; the CLI subprocess reads its own credential store. The variant exists only to record that AgentMux is aware of which provider's ambient creds back this account.

This keeps `~/.claude/credentials.json` as the truth and avoids duplicating secret material into AgentMux's storage.

#### New wstore method

```rust
// wstore.rs (alongside instance_update)
pub fn instance_update_identity(
    &self,
    instance_id: &str,
    identity_id: &str,
    memory_id: Option<&str>,   // None = don't touch memory
) -> Result<(), StoreError>
```

Single UPDATE statement, mirrored to the registry file the way `instance_update` does today. Emits the existing `agentinstances:changed` broker event so the frontend re-renders.

### 5.3 Phase 3 — Frontend banner ("Bind your creds")

For named agents already in the database with `identity_id == ""`, show a single-line banner at the top of the pane:

```
ⓘ Using ambient Claude creds. [Bind to identity →]
```

Clicking opens a small modal:

- If a Claude Identity already exists → "Bind 'Maks' to identity 'Claude — alice@example.com'?" with Confirm.
- If no Claude Identity exists → "Create identity from current Claude credentials?" → backend scans `~/.claude/` (Phase 4: per-provider), creates the Identity + Account + binding, then updates the instance.

The banner is dismissible per-instance (stored in `block.meta["identity_banner_dismissed"]`). Phase 2's runtime auto-bind makes this mostly redundant for *new* logins, but it's the fix for already-orphaned agents like "Maks".

### 5.4 Phase 4 — Multi-provider extension (deferred)

Same pattern for Codex, Aider, Gemini CLI, etc. Each provider needs:

- An `AuthPatternMatch` entry that recognises that CLI's login-success message.
- A naming convention for the auto-created Identity.
- A `SecretRef::AmbientCli { provider }` value pointing to the right ambient store.

The architecture supports it; v1 ships Claude only.

## 6. Schema changes

No new tables. Three additions:

1. `SecretRef::AmbientCli { provider }` variant — JSON-compatible with existing rows (new tag, ignored by old code).
2. `wstore::instance_update_identity` method — single new method, no schema change.
3. `AuthSessionStatus` gains an optional `instance_id` + `block_id` so runtime sessions can be distinguished from pre-launch ones (in-memory field on `AuthSession`, not in SQLite).

## 7. New RPCs

```rust
// New commands in agentmux-srv/src/backend/rpc_types.rs
COMMAND_BIND_INSTANCE_IDENTITY  // body: { instance_id, identity_id, memory_id? }
COMMAND_AUTO_CREATE_IDENTITY    // body: { provider, source_email?, account_name? }
                                // → returns { identity_id }
```

The first is what the banner (§5.3) calls. The second is what the runtime auth-success handler (§5.2) calls.

## 8. Edge cases

| Case | Handling |
|---|---|
| Two named agents share an ambient Claude login | Auto-bind reuses the existing Identity by `(provider, email)` lookup. Both end up bound to the same Identity row. |
| User logs into Claude, then logs out, then logs into a different account | Phase 2 creates a *new* Identity for the new email. The instance gets bound to the new one. The old Identity stays in `db_identities` (orphaned but harmless). |
| Email isn't extractable from CLI stdout | Auto-bind names the Identity `"Claude — (no email)"` with a timestamp suffix. Banner still works as the manual fallback. |
| User explicitly picked "— Blank (no creds) —" | They had a chance to pick. Phase 2 still auto-binds (their intent was likely "I don't have an identity yet" not "never bind one"). Phase 3 banner respects `identity_banner_dismissed` for users who want explicit blank. |
| Multi-version registry rows (`records.len() > 0` path in `forge_handlers.rs:1200`) | The registry-source row's `identity_id` is updated through the same `bundle_identity_upsert` path the SQLite-source one goes through. Phase 2's update propagates both. |
| Pre-launch flow concurrent with runtime session | Pre-launch uses session.into_bundle_id; runtime uses the new finalize_runtime_session. Both go through `bundle_identity_upsert` with idempotent `(provider, email)` dedup. Last-writer-wins is fine. |
| `instance_update_identity` while the instance is running | Subprocess is already spawned with whatever env vars resolved at start. The update takes effect on next spawn. Doesn't disrupt the current session. |

## 9. Phased rollout

| Phase | Scope | LOC | Risk |
|---|---|---|---|
| **1** | Bug A: srv rebuild + frontend defensive fallback | ~5 | Trivial |
| **2** | Runtime AuthSession + auto-Identity creation + `instance_update_identity` | ~250 (Rust) + ~30 (TS) | Medium — touches the auth state machine |
| **3** | Banner UI for already-orphaned agents | ~150 (TS) | Low |
| **4** | Multi-provider extension (Codex, Aider, ...) | Variable | Low — additive |

Phases 1–3 ship together as the next PR. Phase 4 lands per-provider as demand surfaces.

### 9.1 Bug A short-term remediation

Before this spec ships, the user's running `agentmux-srv-0.33.900-windows.x64.exe` binary is missing the `"(ambient creds)"` / `"(vanilla CLI)"` strings due to cargo incremental cache. Workaround for *this* build:

```bash
cargo clean --release  # NOT cargo clean -p — full clean
task package
```

Or wait for the next bump where Phase 1's frontend fallback is in place — the binary's missing strings won't matter.

## 10. Risk

- **Auth state machine complexity.** AuthSession was designed for one-shot pre-launch flows; making it long-lived per-instance adds reentrancy concerns. Mitigation: instance_id key + idempotent finalize.
- **Identity proliferation.** Auto-creating Identities every login could leave hundreds of `"Claude — user@example.com"` rows over time. Mitigation: `(provider, email)` dedup in `bundle_identity_upsert`. Worst case if dedup fails: user can delete via the Identity UI.
- **Privacy: emails in Identity names.** The default name embeds the user's email. For machines with multiple users, that's mostly fine (same user owns AgentMux + CLI creds). Mitigation: allow users to rename Identities; the dedup key is the email, not the name.
- **CLI stdout pattern drift.** If Claude changes its login success message, `record_line` stops detecting it. Mitigation: keep a versioned pattern matrix in `auth_patterns.rs`; banner-based manual binding (§5.3) is the always-available fallback.

## 11. Open questions

- **Q1** Should Phase 2's auto-bind also fire on *first* successful run after binding (i.e. if the user previously ran with an Identity bound but the creds went stale, should we re-bind on re-login)? Lean **no** — that's a different feature ("re-validate identity creds") and conflating them would make the binding non-deterministic.
- **Q2** Should `SecretRef::AmbientCli` be allowed for non-Claude providers without a corresponding listener entry? Lean **no** — the variant should require a registered provider so we can't bind to something the resolver can't represent.
- **Q3** Memory binding (`memory_id == ""`) has the same gap as identity. Spec scope creep if we include it here? Lean **yes, include it** — same fix shape, fewer than 50 LOC additional. Adds a "vanilla CLI" → "auto-named Memory" path. If too much, defer to a sibling PR with this spec as precedent.

---

🤖 Authored by AgentA, 2026-05-15. Implementation lands as a follow-up PR after #878 (cascade-detection) merges. Filed against discussion #707.
