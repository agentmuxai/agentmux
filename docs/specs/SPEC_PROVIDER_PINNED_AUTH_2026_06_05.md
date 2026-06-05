# Spec: Provider-Pinned, Instance-Independent Auth

**Status:** Implementing
**Author:** AgentA
**Date:** 2026-06-05
**Supersedes:** `provider-auth-isolation.md` (2026-03-21, "per-version" model)
**Retro:** `docs/retro/retro-provider-auth-isolation-regression-2026-06-05.md`

---

## Principle

> *Every provider's requirements is a pinned version, completely independent
> from the installed instances.* — directive, 2026-06-05

A provider (claude, codex, gemini, …) is a **first-class, version-pinned unit**:
its CLI binary, config, **and credentials** live in **one** place that does not
depend on, and is not duplicated by, any installed AgentMux instance / channel /
version. Instances **reference** a provider; they never get their own
isolated/recreated copy of its auth.

## Why (the regression this fixes)

The 2026-03-21 design shared auth per *version* (`instances/v{ver}/auth/…`) with
a stated invariant (§4): *the auth check must validate the same dir the agent
runs in.* Two later changes broke it:

- **#850** added a two-phase `CheckCliAuth` whose phase 1 falls back to global
  `~/.claude` while phase 2 validates the isolated dir → "check global / run
  isolated", the §4 hazard.
- **#1027 (channels)** re-rooted config (hence auth) to **per-channel /
  per-data-dir** dirs.

Together: a fresh channel/branch/build starts with an **empty** isolated auth
dir; phase 1 finds global creds, phase 2 validates the empty dir → `loggedIn:false`
forever → a 2 s CLI re-spawn **spin** (CPU pinned, 15 s login timeouts). Full
timeline in the retro.

## The model

```
~/.agentmux/
  shared/                         ← account-wide, version- & channel-independent
    providers/<provider>/         ← DEFAULT provider auth/config (NEW; CLAUDE_CONFIG_DIR, …)
    identities/<bundle>/<provider> ← explicit per-identity multi-account (unchanged)
  instances/v<ver>/cli/<provider> ← pinned CLI binary (already version-shared)
```

- **Single source of truth:** `DataPaths::provider_auth_dir(auth_dir_name)` →
  `shared_dir/providers/<auth_dir_name>`. Every site resolves through it (or the
  equivalent `<home>/shared/providers/<provider>` in cef).
- **§4 restored:** `CheckCliAuth` validates **only** the dir the agent runs in
  (the isolated `CLAUDE_CONFIG_DIR` if set, else global `~/.claude`) — **never**
  "isolated OR global". Phase 2 runs `claude auth status --json` against the same
  dir phase 1 checks.
- **One-time bootstrap:** when the shared provider dir is empty and the user has
  an existing global `~/.claude` login, import it **once**, gated on a
  `.agentmux-cred-seeded` sentinel so a later `claude auth logout` in the provider
  space sticks. This bootstraps the *single shared* dir — it is **not**
  per-instance reseeding.
- **Per-identity layer preserved:** `identity_dir(bundle)/<provider>` still
  overrides `CLAUDE_CONFIG_DIR` for deliberate multi-account bundles.

## Behaviour matrix

| State | Result |
|-------|--------|
| Fresh instance/channel, user logged in globally | Import once → authed; **shared everywhere**, no spin |
| Already authed (shared dir populated) | authed; no CLI spin |
| Logged out in the provider space (sentinel present) | stays not-authed → user logs in; logout **sticks** |
| Never authed anywhere | not-authed (fast, no CLI spawn) → log in once |

## Change surface (this PR)

- `agentmux-common/src/data_paths.rs` — `provider_auth_dir()` + contract test
  (`provider_auth_dir_is_shared_and_channel_independent`).
- `agentmux-cef/src/commands/platform.rs` — `ensure_auth_dir` → shared providers dir.
- `agentmux-srv/src/server/app_api.rs` — agent env `CLAUDE_CONFIG_DIR` → shared providers dir.
- `agentmux-srv/src/server/cli_handlers.rs` — §4 fix (drop global fallback) + one-time import.

## Follow-ups (not in this PR)

1. **Pin the CLI version** — `providers.rs` `pinned_version` is `"latest"` for
   claude; pin to a concrete version (e.g. `2.1.160`) per provider.
2. **Migration sweep** — old per-channel `config/auth/<provider>` dirs are
   orphaned; the import covers the common (globally-authed) case, but a one-time
   move of an existing per-channel login into the shared dir would avoid a
   re-login for users with no global `~/.claude`.
3. **§4 integration test** — assert the dir `CheckCliAuth` validates equals the
   `CLAUDE_CONFIG_DIR` the agent is spawned with (cross-module).
