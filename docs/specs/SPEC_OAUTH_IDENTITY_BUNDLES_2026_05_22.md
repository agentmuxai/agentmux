# SPEC — OAuth credentials as first-class identity-bundle members

**Date:** 2026-05-22
**Author:** AgentA
**Status:** Draft
**Scope:** `agentmux-srv` identity/resolver + storage, agentmux-cef OAuth flow controller, launch modal.
**Related:**
`SPEC_BUNDLE_MANAGEMENT_2026_05_22.md` (bundle CRUD, just shipped),
`SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14.md` (the OAuth flow),
`SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19.md` (the modal reducer),
`docs/analysis/LAUNCH_MODAL_CONTINUE_LOST_2026_05_22.md` (`continueOfId` bug — orthogonal),
`reference_data_dir_unification_plan` (the `~/.agentmux/` unification — coordinates with §4.1).

---

## 1. Summary

Make identity bundles the **single home for all agent credentials** — API keys *and* OAuth — by representing OAuth credentials as a **filesystem pointer** (a per-bundle config directory) with **status tracking**, and making "OAuth → bundle-bound" an **invariant** of the OAuth flow.

Closes the long-standing "ambient OAuth" gap that today forces hacks like the openclaw `authRequired` always-gate (`AgentLaunchModal.tsx:474-477`), lets two "identities" silently share one `~/.claude` directory, and causes the launch modal to misclassify continuation-with-ambient-creds as "needs auth" the moment any state hiccup loses `continueOfId`.

## 2. Motivation

**Today** the identity-bundle system handles only **API-key** secrets. `resolver.rs:46-55`:

```rust
pub fn provider_env_vars(provider: &str) -> Vec<&'static str> {
    match provider {
        "github"    => vec!["GITHUB_TOKEN", "GH_TOKEN"],
        "anthropic" => vec!["ANTHROPIC_API_KEY"],
        "openai"    => vec!["OPENAI_API_KEY"],
        "kimi"      => vec!["MOONSHOT_API_KEY"],
        "aws"       => vec!["AWS_ACCESS_KEY_ID"],
        _ => Vec::new(),
    }
}
```

Resolution is *one thing*: turn a `SecretRef` into a plaintext string and inject it as an env var. There is no path for OAuth. So **OAuth credentials live ambient** — Claude Code at `~/.claude/.credentials.json`, codex / openclaw at their own paths — and identity bundles for OAuth providers are *labels only*: they provide no actual credential isolation.

### Costs

- Two "identities" both reading `~/.claude` are **not actually distinct** — no multi-identity for OAuth providers.
- The launch modal hardcodes `provider?.id === "openclaw"` to **always** gate auth (`AgentLaunchModal.tsx:474-477`) because bundles can't carry openclaw auth. The comment explicitly tags this as Phase δ deferred work.
- Continuation rows with empty `identity_id` fall back to ambient at spawn (`resolver.rs:134-148`) and the launch modal cannot tell *"this user has a working OAuth"* from *"this user has nothing"*.
- The Maks case: continuation correctly skips the auth gate, but any path that loses `continueOfId` (e.g. the `+New identity` round-trip bug) re-engages it — because the system has no first-class representation of "his OAuth is valid."

### Goal

Every credential — API key or OAuth — is **bound to an identity bundle**. Bundles are the truth; ambient is migration glue, not a parallel system.

## 3. Goals / non-goals

**Goals:**

- Identity bundles carry OAuth credentials, as **pointers** (not stored token blobs).
- Per-bundle credential **isolation**: each bundle's Claude/codex/openclaw login is genuinely separate.
- **Status** surfaced on identity (`valid` / `expired` / `needs_reauth` / `unknown`) so users can see and act on credential health.
- A successful OAuth flow **always** lands bound to a bundle — created on the spot if none was selected. The OAuth invariant.
- A **clean migration** for existing ambient users — a "Default" identity bundle that points at the existing `~/.claude`, no token movement required.
- Remove the openclaw `gate always` hack.

**Non-goals:**

- agentmux performing the OAuth refresh dance itself. CLIs (Claude Code, codex, `gh`) refresh their own tokens — agentmux tracks status and offers a **Reconnect** affordance when refresh fails.
- Encrypted secret storage / AWS Secrets Manager backend integration (separate Phase 3 work, already deferred in `resolver.rs`).
- Migrating API-key bindings (they already work — out of scope).
- Cross-machine bundle sync (a different feature).
- The launch-modal `continueOfId` round-trip bug — fixed independently (see the analysis doc).

## 4. Design

### 4.1 Per-bundle credential directories

Each identity bundle owns a credential root:

```
~/.agentmux/identities/<bundle-id>/
    claude/        # CLAUDE_CONFIG_DIR for this bundle
    codex/         # codex's config dir
    openclaw/      # openclaw's config dir
    ...
```

Agents launched with a given identity bundle spawn with the relevant per-provider config-dir env var pointing into this bundle's tree — e.g. `CLAUDE_CONFIG_DIR=~/.agentmux/identities/<id>/claude/`. The CLI reads/writes its tokens there; refreshes are CLI-local; **nothing leaks between bundles**.

*Layout coordinates with the data-dir unification plan* (`reference_data_dir_unification_plan`) — the `identities/` directory hangs off the same unified `~/.agentmux/` root.

### 4.2 `SecretRef` extension

`resolver.rs` `SecretRef` today:

```rust
pub enum SecretRef {
    Env { env_var: String },
    PlaintextDev { plaintext_dev: String },
    SecretsManager { sm_path: String, sm_json_path: Option<String> },
}
```

Add one variant:

```rust
SecretRef::OAuthConfigDir { dir: PathBuf },
```

— a pointer to a directory the agent CLI reads at spawn. **The tokens live in the dir**; agentmux holds only the path. This makes OAuth trivially refresh-safe (the CLI rotates tokens in place; the path is stable).

### 4.3 Provider taxonomy + resolver mode

Today `provider_env_vars` (`resolver.rs:46-55`) maps `provider → env vars`. Replace with a per-provider **classification** that drives the resolution mode:

| Provider     | Class    | Resolution                                              |
|---|---|---|
| `github`     | api-key  | inject `GITHUB_TOKEN`, `GH_TOKEN`                       |
| `anthropic`  | api-key  | inject `ANTHROPIC_API_KEY`                              |
| `openai`     | api-key  | inject `OPENAI_API_KEY`                                 |
| `kimi`       | api-key  | inject `MOONSHOT_API_KEY`                               |
| `aws`        | api-key  | inject `AWS_ACCESS_KEY_ID`                              |
| `claude`     | oauth    | set `CLAUDE_CONFIG_DIR=<bundle>/claude`                 |
| `codex`      | oauth    | set the codex equivalent (its config-dir env var)       |
| `openclaw`   | oauth    | set the openclaw equivalent                             |

`inject_identity_env` (`resolver.rs:115-232`) dispatches by class:

- **api-key** binding → existing path: resolve `SecretRef` → plaintext string → inject env var.
- **oauth** binding → resolve `SecretRef::OAuthConfigDir` → set the provider's config-dir env var to that path.

The per-binding *failure-is-skipped* semantic is preserved across both modes.

### 4.4 Status semantics

`IdentityAccount.status: String` exists in the schema today, populated as `"unknown"` (`resolver.rs:258`). Define values for **oauth** accounts:

| Value           | Meaning                                                                 |
|---|---|
| `valid`         | Token present and (probed) not expired.                                 |
| `expired`       | Access token expired; refresh likely succeeds. Soft state.              |
| `needs_reauth`  | Refresh token rejected / missing; user must Reconnect. Hard state.      |
| `unknown`       | Not yet probed. Initial state on bundle import.                         |

**Update points:**

- On **bind** (initial OAuth success) → `valid`.
- On **agent spawn** → cheap expiry-read of `bundle/<provider>/credentials.json` updates status (no network round-trip).
- On user-triggered **Refresh status** → re-probe.
- On a **failed spawn** where the CLI reports auth error → `needs_reauth`.

**Surfaces:**

- The **Identity & Memory** manager (the hamburger modal from `BundleManagerModal`) shows status per-account with a **Reconnect** button when `needs_reauth`.
- The launch modal's auth panel — instead of "Connect to Claude Code" out of nowhere — says e.g. *"Claude credentials need reconnecting"* when the bound account's status is `needs_reauth`.

### 4.5 OAuth-success invariant

PR #969 (Feature 1) made OAuth Connect open the New Identity modal first. This spec makes it an **invariant**:

> A successful OAuth flow MUST land bound to an identity bundle. If none is selected when OAuth begins, one is created (user- or auto-named) *before* the OAuth process starts.

Sequence (`AuthFlowController` orchestrates):

1. Bundle selected, or new bundle named + created.
2. Bundle's `<provider>/` dir allocated under `~/.agentmux/identities/<id>/`.
3. The provider's CLI process spawned with the config-dir env var pointing at that path.
4. CLI runs its OAuth dance and writes its own tokens into the bundle dir.
5. On success, the `db_identity_bindings` row is persisted with the account's `SecretRef::OAuthConfigDir { dir }` and status `valid`.
6. On failure, the dir is left intact for the user's next attempt; the bundle row stays — the binding does not.

### 4.6 Spawn wiring

`inject_identity_env` is already the call-site for "build the agent's env from its identity." With this spec, **oauth-class bindings** also inject the per-provider config-dir env var. The agent CLI then reads its tokens from the per-bundle dir transparently — no further wiring needed in the agent runtime.

## 5. Migration

### 5.1 The "Default" identity bundle — closes the Maks case for real

On first run under the new resolver — or as a one-shot startup migration:

1. If `~/.claude/.credentials.json` exists *and* no identity bundle has a `claude` binding, create an identity bundle named **"Default"** (or the OS user's display name).
2. Its `claude` binding stores `SecretRef::OAuthConfigDir { dir: <user-home>/.claude }`.
3. Status is probed (`valid` / `expired` / `needs_reauth`).
4. Any `db_agent_instances` row with empty/`"blank"` `identity_id` is back-filled to this Default bundle.

**Result:** Maks's existing OAuth is now bound to a real bundle. No token movement, no fabrication — the pointer genuinely resolves. The launch modal sees a valid bound binding and behaves correctly *with or without* `continueOfId`. Anyone else with an ambient login converts the same way.

### 5.2 codex / openclaw

Same treatment when those providers' config dirs are detected on disk.

### 5.3 Backward compat

Empty `identity_id` continues to be tolerated by the resolver (the existing ambient fallback at `resolver.rs:134-148`, with its warn-log, stays). Migration makes empty rare; not eliminating the fallback is intentional — it's the safety net for any future ambient launch we haven't migrated.

## 6. Removed hacks

- `AgentLaunchModal.tsx:474-477` — drop the `provider?.id === "openclaw"` always-gate. With bundles carrying openclaw auth, the standard `hasMatchingBinding` path handles it.
- The `"blank"` sentinel sub-checks become rare (migration back-fills most ambient launches).
- `inject_identity_env`'s warn-log for empty `identity_id` (`resolver.rs:141-147`) keeps firing only for genuinely-unmigrated rows — a useful signal that the migration missed something.

## 7. Rollout

Suggested PR sequence — each PR independently mergeable, each adds value alone:

| PR | Scope |
|---|---|
| **A** | Schema: `SecretRef::OAuthConfigDir` variant + storage migration. Per-bundle dir layout (`~/.agentmux/identities/<id>/`) provisioned on bundle create. |
| **B** | Resolver: provider-class table replaces `provider_env_vars`; `inject_identity_env` learns the oauth-resolution mode (sets `CLAUDE_CONFIG_DIR` etc.). API-key path unchanged. |
| **C** | OAuth-success invariant: `AuthFlowController` spawns the CLI with the bundle's config dir; auto-create-bundle path on OAuth-without-identity; persists the `OAuthConfigDir` binding on success. |
| **D** | Status field semantics: cheap expiry-probe + UI surfacing in the Identity & Memory manager (Reconnect button) + launch-modal auth-panel wording. |
| **E** | Migration: seed Default bundle from `~/.claude`; back-fill empty `identity_id` rows; symmetric handling for codex / openclaw config dirs. |
| **F** | Cleanup: drop the openclaw always-gate; tighten the launch modal's auth gate to the bound-binding check alone. Update the `Phase δ` comment to "done." |

## 8. Open questions

1. **Codex / openclaw config-dir env vars** — confirm the exact env var names for each CLI. Default to documented values; verify by probe before locking in the matrix.
2. **Bundle deletion** — should removing an identity bundle wipe its credential dir? Yes (it's a credential rotation event), but with a confirm modal (`BundleManagerModal` already has the framing).
3. **Multiple OAuth accounts per provider per bundle** — out of scope for this spec (one bundle ⇒ one `<provider>/` dir per provider). If we want "two Claude logins in one bundle" later, the dir layout becomes `<provider>/<account-id>/` — additive change, no schema break.
4. **Status probing scope** — `valid`/`expired` is read from token expiry on disk. `needs_reauth` requires either a CLI auth-fail signal or a network probe. Defer the network probe; rely on spawn-time CLI feedback initially.

## 9. Why this is worth the spend

The current "API keys in bundles, OAuth ambient" split is a long-standing seam — it forced #969 to land the OAuth-creates-bundle UX without an actual backend representation, it gave the openclaw gate-always hack a permanent home, it made the Maks-case dance subtle and fragile. Closing the seam:

- Removes a class of "the modal forgot I'm authed" bugs (Maks's symptom is *one* of them).
- Makes the bundle manager (#972) functionally meaningful for OAuth users, not just API-key ones.
- Aligns with the data-dir unification direction (one tree, one source of truth).
- Drops the deferral hedge in the launch modal — the codebase stops pointing at "Phase δ" and just *is* the Phase δ.
