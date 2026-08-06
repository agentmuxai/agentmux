# Spec: Opt-in isolated auth for `task dev` testing

**Date:** 2026-07-27
**Status:** Implemented — defaults amended by `docs/specs/SPEC_ISOLATED_AUTH_DEFAULT_BY_CHANNEL_2026_08_06.md` (2026-08-06)
**Related:** `docs/specs/REPORT_LOGIN_PERSIST_FAILURE_AND_STUCK_WORKING_2026_07_27.md`,
`docs/retro/retro-provider-auth-isolation-regression-2026-06-05.md`

> **Amended 2026-08-06:** everything below describing the mechanism
> (channel-scoped store, credential-dir isolation, the two load-bearing
> migration fixes) is still accurate and authoritative. What changed is
> the *default* — "When unset (the default): zero behavior change" and
> "Isolation must never be the default" (§ below) are no longer true for
> any channel except `stable`. See
> `docs/specs/SPEC_ISOLATED_AUTH_DEFAULT_BY_CHANNEL_2026_08_06.md` for
> the current default and why it changed.

## The incident that motivated this

Armory identity accounts (`db_accounts`, `db_agent_identity_links` — Claude/
Anthropic OAuth tokens, API keys, etc.) live in exactly ONE global,
channel-independent store: `~/.agentmux/shared/store.db` plus
`~/.agentmux/shared/identities/<account_id>/` for the on-disk OAuth
credential directories. This store is shared by **every** `task dev`
branch, every portable build, every instance on the machine — there was
no isolation at all.

While manually testing a destructive Armory flow ("delete an account") in
a `task dev` test branch, the destructive delete removed the same account
backing a live, unrelated Claude Code agent session on the same machine.
Nothing about the test branch was isolated from the real, in-use
credential store.

## What this adds

An opt-in env var, `AGENTMUX_ISOLATED_AUTH=1`. When set for a `task dev`
process, that instance gets its own fully isolated identity store and
credential directory tree, colocated under its own channel dir
(`instance_dir`) — safe to delete accounts from, impossible to affect any
other instance's real credentials.

**When unset (the default): zero behavior change.** This is deliberate.
Re-authenticating every provider on every branch switch is real daily
friction — it's the whole reason the seed-from-global login tier exists
in the first place. Isolation must never be the default.

## Explicit non-goals

- **Does not touch `provider_auth_dir()`** (the ambient/no-account-bound
  default provider config dir, e.g. `~/.agentmux/shared/providers/claude/`).
  That directory's global, channel-independent nature is the structural
  fix for a real prior regression — see
  `docs/retro/retro-provider-auth-isolation-regression-2026-06-05.md` ("the
  validate-spin bug"). This feature is orthogonal to that fix: the
  Armory-account-bound login path always resolves `CLAUDE_CONFIG_DIR` from
  `identities_dir()`, never from `provider_auth_dir()` — confirmed by
  reading `CheckCliAuthCommand`'s implementation, which reads
  `auth_env["CLAUDE_CONFIG_DIR"]` identically regardless of isolation mode.
  `provider_auth_dir()` is reached only on the true ambient/no-account-bound
  path, which this feature never touches.
- **Does not change default `task dev` behavior at all.** Every existing
  test, every existing workflow, is byte-for-byte unaffected when the flag
  is unset.
- **Does not isolate agent definitions, the agent registry, or
  transcripts.** Those are separate subsystems (`resolve_shared_registry_dir`,
  `resolve_shared_definitions_dir`, `resolve_shared_transcripts_dir`) with
  their own already-correct cross-channel-sharing semantics, untouched by
  this flag.

## What becomes isolated vs. what stays global

| Isolated (when flag is set) | Stays global (always) |
|---|---|
| `db_accounts`, `db_agent_identity_links` | Agent registry (`~/.agentmux/shared/agents/registry/`) |
| Memory bundles, drone definitions, MuxBus credentials (same `shared_store`/`id_store` file) | Agent definitions (`~/.agentmux/shared/agents/definitions/`) |
| OAuth credential directories (`identities_dir()`) | Global transcripts (`~/.agentmux/shared/agents/transcripts/`) |
| | Ambient/default provider auth dir (`provider_auth_dir()`) |

Memory bundles, drone definitions, and MuxBus credentials are isolated as
a side effect of `shared_store`/`id_store` being one physical file with
one migration-tracking table — they aren't separable without a larger,
riskier surgery, and isolating them too is arguably correct anyway: a
disposable test channel shouldn't depend on or pollute real shared
bundles/drones either.

## How it works

`id_store` (`bootstrap.rs`) isn't a separate database — it's an `Arc<Store>`
alias that points at `shared_store` when available and migrated, else
falls back to the per-channel `wstore`. Two functions decide where
`shared_store` physically lives:

1. **`agentmux_common::isolated_auth_enabled()`** — the single source of
   truth for the flag (`AGENTMUX_ISOLATED_AUTH == "1"`). One helper, read
   fresh at every call site, rather than three independent env-var reads
   that could drift out of sync.
2. **`DataPaths::identities_dir()`** — returns `instance_dir.join("identities")`
   instead of `shared_dir.join("identities")` when isolated.
3. **`registry::resolve_shared_store_path()`** — returns
   `instance_dir.join("identity-store.db")` (reading `AGENTMUX_INSTANCE_DIR`,
   already exported to every child process) instead of the global
   `store.db` path when isolated. Falls back to the global path if
   `AGENTMUX_INSTANCE_DIR` is unresolvable (e.g. a bare `cargo run` outside
   the launcher).

### The two load-bearing fixes this required

**Migration `ctx.home` must stay anchored to the true global root,
independent of isolation.** `migrations/runner.rs`'s `MigrationContext.home`
used to be derived by walking up from `shared_store_path`
(`.parent().parent()`), which assumed the shared store was always exactly
`<home>/shared/store.db`. Once `resolve_shared_store_path()` can return an
isolated path, that arithmetic would silently produce the wrong `home` for
every other Global migration (registry/definitions/transcripts dirs,
backups, the error log) — and would make migration `0011_shared_store_backfill`'s
cross-channel sibling scan resolve the wrong root entirely. Fixed by adding
`resolve_home()` (a single shared helper, not one inline derivation per
call site) that calls `registry::resolve_global_shared_root()` directly —
a function deliberately **unaffected** by the isolation flag.

**Migration `0011_shared_store_backfill` must skip its cross-channel
sibling scan when isolated.** With `ctx.home` correctly anchored, this
migration would otherwise correctly find and backfill every other
channel's REAL `db_accounts`/`db_agent_identity_links` rows into the new
isolated store on its first boot — defeating the entire "starts genuinely
empty" point of this feature. This channel's own local objects.db is still
included (that's this channel's own data, fine to carry in); only the
scan across every *other* channel/branch on the machine is skipped.

## First-boot behavior

The first time an isolated store boots for a given channel, every
currently-registered Global-scope migration runs once against it (migration
state is tracked per-file in `db_migrations`) — this is expected and
desirable: the isolated store ends up looking like a normal fresh install
(starter skills/MCP servers seeded, etc.), not a broken one. Only
migration 0011's cross-channel backfill is intentionally skipped, per above.

## How to opt in

Set the env var directly in your shell before invoking `task dev` — do
**not** add a Taskfile `env:` block for this.

```bash
# bash / Git Bash
AGENTMUX_ISOLATED_AUTH=1 task dev
```

```powershell
# PowerShell
$env:AGENTMUX_ISOLATED_AUTH=1; task dev
```

**Why not a Taskfile `env:` block:** Task's own declarative `env:` key does
NOT cascade across a nested `- task: X` invocation on Windows — this is a
real, already-hit bug in this exact `Taskfile.yml`. `AGENTMUX_DEV=1` had to
be inlined directly onto `dev:serve`'s shell command line instead of
relying on the `dev` task's own `env:` key for exactly this reason (see the
comment at `Taskfile.yml` around the `dev:serve` task). Direct
shell-env-var usage is ambient-env inheritance, not Task's `env:`
cascading, so it isn't subject to that bug and requires zero Taskfile
changes. If a discoverable `task dev:isolated-auth` convenience target is
ever added, it must inline the var onto `dev:serve`'s command line the same
way `AGENTMUX_DEV` already does — not rely on a YAML `env:` block.

## Verification

1. `cargo test -p agentmux-srv && cargo test -p agentmux-common` — all
   tests pass, including the isolation-specific ones:
   - `agentmux-common`: `data_paths::tests::identities_dir_is_shared_by_default`,
     `identities_dir_is_per_channel_when_isolated_auth_set`
   - `agentmux-srv`: `registry::paths::tests::shared_store_path_default_is_global`,
     `shared_store_path_isolated_uses_instance_dir`,
     `shared_store_path_isolated_without_instance_dir_falls_back`,
     `migrations::runner::tests::home_is_invariant_to_isolated_auth`,
     `migrations::m0011_shared_store_backfill::tests::backfills_sibling_accounts_when_not_isolated`,
     `skips_sibling_accounts_when_isolated`
2. Boot `AGENTMUX_ISOLATED_AUTH=1 task dev` on a scratch branch. Confirm
   via `muxlog srv grep "shared store"` that the log line reads
   `"shared store: attached (ISOLATED — channel-scoped)"` with a path
   under `dev/<branch>/.../identity-store.db`.
3. Create an Armory account in that isolated boot. Confirm it does NOT
   appear in a normal (`task dev`, no flag) boot of the same or a
   different branch, and vice versa.
4. Delete that isolated account — confirm the real global store/credentials
   (`~/.agentmux/shared/store.db`, `~/.agentmux/shared/identities/`) are
   completely untouched.
5. Confirm a normal `task dev` (no flag) boot is unaffected: same global
   store, same account list as before this feature shipped.
