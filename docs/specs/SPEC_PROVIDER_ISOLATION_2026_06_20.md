# SPEC: Provider environment isolation — never touch the user's `~/.claude` or global CLI

**Status:** Approved — implementing auth half
**Author:** AgentA
**Date:** 2026-06-20
**Extends / hardens:** `SPEC_PROVIDER_PINNED_AUTH_2026_06_05.md` (#1291), `provider-auth-isolation.md` (2026-03-21)
**Diagnosis:** `docs/reports/REPORT_AGENT_AUTH_DIVERGENCE_2026_06_20.md`
**Related:** `SPEC_HOST_CLI_LOGIN_CAPTURE_2026_06_20.md` (the login mechanism), retro `retro-provider-auth-isolation-regression-2026-06-05.md`

---

## 0. Directive

> *We designed AgentMux so each provider keeps its own auth. I do not want to
> pollute a user's environment. Same applies to the executable.* — 2026-06-20

AgentMux must be a **self-contained sandbox** for every provider. It may **read**
the user's environment once, with consent, to import an existing login — but it
must **never run from, refresh into, or otherwise mutate** the user's personal
`~/.claude` (etc.) or the user's globally-installed CLI.

## 1. Hard invariant

For every provider `P` and every agent:

- **INV-A (auth):** the agent's live credential dir (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, …) is an **AgentMux-owned** path under `~/.agentmux/…`. It is **never** the user's `~/.<P>` dir. Tokens are read and **refreshed in the AgentMux dir only**.
- **INV-X (executable):** the agent runs an **AgentMux-installed, version-pinned** CLI under `~/.agentmux/…`. It **never** resolves or runs the user's PATH/global CLI.
- **INV-R (read-only import):** the user's `~/.<P>` may be **read** exactly once, on explicit/opt-in import, and **copied** into the AgentMux dir. It is never written, never pointed-at as the live dir, never the run target.
- **INV-M (memory & state):** an agent's **auto-memory** (`projects/<repo>/memory/`), **user `CLAUDE.md`**, **transcripts/sessions**, and **settings** follow `CLAUDE_CONFIG_DIR` — they live in the AgentMux dir, **never** `~/.<P>`. This holds *automatically* once INV-A is satisfied (it is the same dir — `CLAUDE_CONFIG_DIR` relocates the whole Claude home, not just credentials), but is stated so it can't silently regress. (Org-managed policy `CLAUDE.md` at a system path is read-only org config, not user state — out of scope.)

A reviewer seeing any code that sets a `*_CONFIG_DIR`/`*_HOME` to `~/.<P>`, or runs a CLI resolved via `where`/`which`, must reject it.

## 2. Current violations (verified)

| Axis | Violation | Where | Commit |
|---|---|---|---|
| Auth | Default identity bundle's account `secret_ref` = `OAuthConfigDir { dir: ~/.claude }` — a **live pointer** to the user's global dir; "the CLI keeps reading + refreshing in place" | `agentmux-srv/src/identity/migration.rs` `run_default_bundle_migration` (account creation ~l.244-258) | #983 |
| Auth | Two-phase `CheckCliAuth` phase-1 falls back to global `~/.claude` | `agentmux-srv/.../cli_handlers.rs` | #850 |
| Exec | `detect_cli` resolves the CLI via `where`/`which <provider>` → the user's global binary; launch falls back to it when nothing is installed isolated | `agentmux-cef/src/commands/providers.rs` `detect_cli` (l.141), `get_cli_path` (l.514) | — |
| Exec | `pinned_version` for claude is `"latest"` — not actually pinned | `agentmux-cef/src/commands/providers.rs` (`CLAUDE_VERSION`) | — |

The #1291 shared-dir model (`~/.agentmux/shared/providers/<provider>/`) is correct
and partly in place (`ensure_auth_dir`), but the #983 bundle pointer **overrides**
it for any agent bound to an identity (today: all of them, via `default`). See the
diagnosis report §2 (Nark resolves to `~/.claude`; Poal froze on the shared dir).

## 3. The model

```
~/.agentmux/
  shared/
    providers/<provider>/.credentials.json     ← DEFAULT live auth dir (AgentMux-owned)   [INV-A]
    providers/<provider>/.agentmux-cred-seeded  ← bootstrap sentinel (import-once gate)    [INV-R]
    identities/<bundle>/<provider>/…            ← explicit multi-account bundles (own dirs) [INV-A]
  <cli-root>/cli/<provider>/node_modules/.bin/  ← pinned, AgentMux-installed CLI            [INV-X]

~/.claude  (and ~/.codex, …)                    ← USER's — read-once on import only, never written
```

- **Single auth source of truth:** `DataPaths::provider_auth_dir(<auth_dir_name>)` → `~/.agentmux/shared/providers/<auth_dir_name>`. The Default bundle's account `dir` is **this**, never `~/.<provider>`.
- **§4 restored:** `CheckCliAuth` validates *exactly* the dir the agent runs in (the AgentMux dir), never "isolated OR global".
- **Per-identity bundles** keep their own AgentMux-owned dirs for deliberate multi-account.

## 4. Fix — AUTH half (this PR)

### 4.1 Default bundle points at the AgentMux dir + copies creds (the #983 reversal)
In `run_default_bundle_migration`, when ambient creds exist:
1. Compute `dest = DataPaths::provider_auth_dir(auth_dir_name)` (e.g. `~/.agentmux/shared/providers/claude`).
2. **Copy** `~/.<auth_dir_name>/.credentials.json` → `dest/.credentials.json` (read-only import), gated by a `dest/.agentmux-cred-seeded` sentinel so a later logout in the AgentMux dir sticks and we don't re-import.
3. Create the account with `secret_ref: OAuthConfigDir { dir: dest }` — **not** the ambient dir.

Result: agents refresh in `dest`, never in `~/.claude`. (INV-A, INV-R.)

### 4.2 Migration sweep — un-pollute existing installs
Existing Default accounts created by #983 already point at `~/.<provider>`. A one-time sweep: for any `default` account whose `secret_ref.dir` is the user's ambient `~/.<provider>`, **repoint** it to `dest` (copying creds first if `dest` is empty). This fixes already-bound agents (Nark, Poal) without a manual re-login. Idempotent; sentinel-gated.

### 4.3 Re-resolve, don't trust the frozen launch env
An agent's `CLAUDE_CONFIG_DIR` is frozen into block-meta `cmd:env` at first launch and reused on every turn + on "Login Again" (report §2.2). After 4.1/4.2 the binding is correct but already-launched agents carry a stale frozen dir. On **re-auth** (and on spawn), re-resolve `CLAUDE_CONFIG_DIR` from the identity binding (`inject_identity_env`) and refresh `cmd:env`, so a stale agent picks up the AgentMux dir. (This is the DRY hinge: create and re-auth resolve the **same** way.)

### 4.4 §4 check fix
Confirm/keep `CheckCliAuth` validating only the agent's resolved AgentMux dir — drop any global `~/.claude` fallback (the #850 hazard). Add the cross-module invariant test: the dir `CheckCliAuth` validates == the `CLAUDE_CONFIG_DIR` the agent is spawned with.

### 4.5 `seed_provider_auth_from_global` (#1613) → explicit opt-in import, targeted
Reframe the 🌐 path as the *manual* form of 4.1's import (read `~/.claude` → copy into the agent's **resolved** dir). Two changes:
- Target the agent's **resolved** `CLAUDE_CONFIG_DIR` (passed by the frontend), not the hardcoded `shared/providers/claude` (report §5).
- It remains read-only w.r.t. global and is never the default recovery — primary recovery is a fresh login into the AgentMux dir (§5 below / login-capture spec).

## 5. Fix — EXECUTABLE half (follow-up PR)

- **Ensure-pinned-install on launch:** if the pinned CLI isn't installed under `~/.agentmux/.../cli/<provider>`, install it (existing `install_cli`); run **only** that path.
- **Drop the global run fallback:** remove the `where/which <provider>` resolution as a *run* target (it may survive only as an "import an existing login" hint, never executed).
- **Pin the version:** replace `CLAUDE_VERSION = "latest"` with a concrete pinned version per provider.

(Out of scope for this PR; tracked here so the executable axis isn't lost.)

## 5b. Memory & state (config half — already covered by INV-A, with one optional knob)

`CLAUDE_CONFIG_DIR` relocates the **entire** Claude home, so memory/transcripts/
settings ride along with the auth dir for free (INV-M). Confirmed on disk: the
AgentMux provider dir already contains `.claude.json`, `projects/`, `sessions/`,
`backups/` — not just `.credentials.json`. So **the auth-half fix isolates memory
too**; nothing extra is required for correctness.

- **Auto-memory** → `<CLAUDE_CONFIG_DIR>/projects/<repo>/memory/MEMORY.md`.
- **User `CLAUDE.md`** → `<CLAUDE_CONFIG_DIR>/CLAUDE.md`.
- **Project `CLAUDE.md`** → the agent's working dir (`~/.agentmux/agents/…`).

**Optional knob — pin memory explicitly.** Claude Code honours
`autoMemoryDirectory` in `settings.json` (absolute or `~/`-prefixed, any settings
scope) to put auto-memory at a chosen path *independent of* `CLAUDE_CONFIG_DIR`.
If we ever want memory shared per-provider or kept per-agent separately from the
auth dir, set `autoMemoryDirectory` in the agent's isolated `settings.json` rather
than relying on the implicit `projects/<repo>/memory/` location. (Not required;
a deliberate-control lever for the config half.) Full analysis:
`docs/reports/REPORT_AGENT_AUTH_DIVERGENCE_2026_06_20.md` §10.

## 6. Behaviour matrix (post-auth-half)

| State | Result |
|---|---|
| Fresh install, user logged in globally | Import `~/.claude` → `shared/providers/claude` **once** → authed in the AgentMux dir; `~/.claude` untouched thereafter |
| Existing agent pointed at `~/.claude` (Nark/Poal) | Sweep repoints to the AgentMux dir (copy-once) → next spawn/re-auth authed, isolated |
| Logged out in the AgentMux dir (sentinel present) | stays logged out → user logs in **into the AgentMux dir**; logout sticks; global never re-imported silently |
| No global login | not-authed → log in once **into the AgentMux dir** (setup-token / paste-code per login-capture spec) |
| User refreshes/rotates their personal `~/.claude` | no effect on AgentMux; the two are decoupled after import |

## 7. Change surface — auth half

- `agentmux-srv/src/identity/migration.rs` — §4.1 (copy + point at AgentMux dir, sentinel) and §4.2 (sweep existing ambient-pointing accounts).
- `agentmux-common/src/data_paths.rs` — reuse `provider_auth_dir`; add a credential-copy + sentinel helper if shared.
- `agentmux-srv/src/identity/resolver.rs` / `server/cli_handlers.rs` — §4.3 re-resolve on re-auth; §4.4 §4 check (verify it's already free of the global fallback; add the invariant test).
- `agentmux-cef/src/commands/providers.rs` (`seed_provider_auth_from_global`) + frontend `seed-global-login.ts` — §4.5 target the resolved dir; keep as opt-in import.

## 8. Invariants / tests

- **T-A1:** after migration with an ambient `~/.claude` present, the Default `claude` account `secret_ref.dir` is under `~/.agentmux/…` and **not** `~/.claude`; `~/.claude/.credentials.json` is byte-unchanged (read-only).
- **T-A2:** sweep repoints a pre-existing account whose `dir == ~/.claude` to the AgentMux dir, copying creds, idempotently (sentinel).
- **T-A3 (§4):** the dir `CheckCliAuth` validates == the agent's spawned `CLAUDE_CONFIG_DIR`.
- **T-A4:** a token refresh by an agent writes under `~/.agentmux/…`, never `~/.claude` (observed: `~/.claude` mtime unchanged across a refresh).

## 9. Open decisions

- **Migration smoothness:** §4.2 auto-imports (read-only copy) so existing globally-authed users don't break. The clean long-term state is a fresh login into the AgentMux dir; the copy shares a `refreshToken` with global until then (rotation could stale one side — acceptable for a one-time bridge; documented).
- The shared `refreshToken` caveat applies equally to the 🌐 import (§4.5).

## 10. Implementation log

- **2026-06-20 — auth half landed.**
  - **§4.1 + §4.2 (`agentmux-srv/src/identity/migration.rs`):** the Default
    bundle account now points at the AgentMux dir (`provider_auth_dir`), with the
    ambient `~/.claude` cred **copied** in once (`import_ambient_once`,
    sentinel-gated) — reversing the #983 pointer-to-ambient. A
    `sweep_default_accounts_off_ambient` pass repoints pre-existing
    ambient-pointing accounts (Nark/Poal). Tests `T-A1`
    (`ambient_claude_creds_create_default_bundle_and_bind`, updated) and `T-A2`
    (`sweep_repoints_existing_ambient_account_to_agentmux_dir`, new) green;
    8/8 migration tests pass.
  - **§4.3 — satisfied server-side (key finding).** `inject_identity_env_async`
    runs at **every** agent turn spawn (`app_api.rs` AgentSend,
    `agent_handlers.rs` AgentInput) and **overwrites** `CLAUDE_CONFIG_DIR` from
    the binding (`resolver.rs:563` `env_vars.insert(...)`). So the migration
    fix propagates to every turn automatically — a stale frozen `cmd:env`
    (e.g. Nark's `~/.claude`) is overridden by the binding's AgentMux dir on the
    next turn. No frontend re-resolve needed for turn traffic. **Residual:** the
    `runCliLogin` re-login path (a cef host command) still writes to the frozen
    `cmd:env`; that's the un-scrapeable v2.1.x login anyway, so the credential
    **write target** is folded into `SPEC_HOST_CLI_LOGIN_CAPTURE` (setup-token /
    paste-code must write to the resolved dir), not duplicated here.
  - **§4.4 — verified already-correct (#1291).** `cli_handlers.rs` `CheckCliAuth`
    validates the dir the agent runs in (`cmd.auth_env.CLAUDE_CONFIG_DIR`, global
    only as an absent-fallback), and the one-time global→isolated import targets
    that same resolved dir. No "check global / run isolated" split remains.
  - **§4.5 done (`providers.rs` + `cef-api.ts` + `custom.d.ts` +
    `seed-global-login.ts` + `useAgentControllerStatus.ts`):** the 🌐 seed now
    targets the agent's **resolved** `CLAUDE_CONFIG_DIR` (passed from `cmd:env`),
    **guarded** to paths under `~/.agentmux` — a stale `~/.claude` is rejected →
    shared default. The seed can never write the user's personal env (INV-R),
    and bundle agents seed into their own dir.
  - **Net:** after this lands + next srv startup, no agent turn reads/writes the
    user's `~/.claude`; existing agents auto-recover via the sweep + per-spawn
    re-resolution; the 🌐 import is hardened. Executable axis (§5) remains a
    follow-up.
