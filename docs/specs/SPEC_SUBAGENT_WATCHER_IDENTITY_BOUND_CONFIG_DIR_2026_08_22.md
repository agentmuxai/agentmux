# SPEC: subagent_watcher watches the identity-bound Claude config dir, not a stale spawn-time snapshot

**Date:** 2026-08-22
**Status:** Implemented
**Author:** Korp
**Repos touched:** `agentmux` (`agentmux-srv/src/identity/resolver/inject.rs`, `agentmux-srv/src/backend/subagent_watcher/parse.rs`, `agentmux-srv/src/server/reactive.rs`, `agentmux-srv/src/server/service/misc.rs`)
**Diagnosis:** live repro during the follow-up to `docs/reports/REPORT_MUXSPECT_MUXLOG_CROSS_CHANNEL_INSPECTION_2026_08_22.md` §4 (the original "dispatched subagent never appears in Swarm" bug)
**Related:** `SPEC_PROVIDER_ISOLATION_2026_06_20.md` §4.3 (the frozen-`cmd:env` tradeoff this closes a gap in), `docs/retro/retro-auth-isolation-invariant-silently-orphaned-2026-07-14.md`

## 1. Root cause

`subagent_watcher::resolve_claude_config_dir` (`parse.rs:493`) resolves the
directory to watch for a block's subagent JSONL files by reading the block's
persisted `cmd:env.CLAUDE_CONFIG_DIR` meta — a snapshot written **once**, at
`agent_open.rs:307-311`, using the **generic** shared-provider auth dir
(`~/.agentmux/shared/providers/claude`).

For an agent bound to an **explicit Armory identity**, that snapshot is
wrong from the moment it's written. `SPEC_PROVIDER_ISOLATION_2026_06_20.md`
§4.3 already documents this exact staleness and explicitly accepted it for
its own purposes: `inject_identity_env_with_broker` re-resolves and
**overwrites** the live spawn env's `CLAUDE_CONFIG_DIR` from the identity
binding on **every turn** — so the actual CLI process always runs with the
correct, identity-bound dir, and auth correctness never depended on
`cmd:env` staying fresh. `subagent_watcher` was added later and — reasonably,
at the time — trusted `cmd:env` as if it were that same resolved value. It
isn't. `cmd:env` is a write-once launch-time snapshot; the real per-turn
value lives only in the identity resolver.

**Confirmed live** on a real running instance (this agent, `Korp`, on a
per-build channel): a genuine `Agent`-tool dispatch's transcript was written
to `.../shared/identities/<uuid>/claude/projects/.../subagents/agent-<id>.jsonl`
(the identity-bound dir Claude CLI actually used), while `subagent_watcher`
had registered against `.../shared/providers/claude/projects` (the generic
dir `cmd:env` recorded at launch) — confirmed via the srv log
(`muxlog swarm`, this session's own Ext 6) showing zero lines ever
mentioning this block, and via `muxspect`/filesystem inspection showing the
watcher's own "watching for subagent JSONL files" event fired once, at
registration, against the wrong directory, and never fired again. This
supersedes the original report's leading suspect
(`session_belongs_to_block`) — that gate is downstream of the file watch and
never runs, because the watch itself never sees the write.

This is not a race or a rare edge case: it reproduces for **every** agent
with an explicit (non-ambient) identity binding, deterministically, forever
— which, per `docs/reports/REPORT_SWARM_DISPATCH_ATTRIBUTION_AND_LIFECYCLE_2026_08_19.md`'s
own framing, matches "a dispatched subagent never appearing in Swarm at
all" far better than a naming/attribution bug would.

## 2. Fix

Added `resolve_bound_oauth_config_dir` in `identity/resolver/inject.rs` — a
new, **pure, read-only** helper, deliberately **not** a call to
`inject_identity_env`/`inject_identity_env_with_broker`. Reusing those
wholesale was considered and rejected: they resolve API-key secrets into
the caller's env map (a real leak risk for a caller that only wants a
directory path), perform a token-expiry filesystem probe, upsert account
status to the DB, and publish `identityaccounts:changed` — none of which
belong on every agent **registration**, only on an actual spawn/turn. The
new helper does only the minimal lookup the OAuth branch of
`inject_identity_env_with_broker` does: instance → definition's effective
provider (`resolve_effective_provider_id` + `resolve_provider_alias`, must
be OAuth-class) → direct binding for that provider
(`resolve_bindings_for_instance`, `broker: None` — no diagnostic publish
from a background path) → account → `SecretRef::OAuthConfigDir.dir`. Any
failure at any step (no instance, no def, not OAuth-class, no binding, bad
secret_ref) returns `None` — never blocks, never logs a misleading
"spawn refused" (this isn't a spawn).

`subagent_watcher::resolve_claude_config_dir` gained an optional
`bound_dir: Option<PathBuf>` parameter, tried **first**; falls back to the
existing `cmd:env` / `derive_claude_config_dir` chain unchanged when it's
`None` (ambient/unbound agents keep exactly their old behavior — this is
purely additive for the identity-bound case). Both call sites
(`server/reactive.rs`'s register handler, `server/service/misc.rs`'s
equivalent) now compute `resolve_bound_oauth_config_dir` first and pass it
through.

## 3. Non-goals

- Does not touch `inject_identity_env`/`inject_identity_env_with_broker`
  themselves, or their side effects (probe/upsert/publish) — those are
  correct as-is for the spawn path; this is purely a second, independent,
  side-effect-free reader of the same underlying binding data.
- Does not backfill/refresh an already-registered agent's watch dir if its
  identity binding changes mid-session (e.g. re-auth to a different
  account) — registration is still a one-shot `watch_agent` call, same as
  before. A rebind mid-session was already a gap for `cmd:env` itself
  (`SPEC_PROVIDER_ISOLATION_2026_06_20.md` §4.3's own scope); closing it
  fully would mean re-registering the watch on every identity change, a
  larger change than this fix's scope. Flagged as a real follow-up, not
  silently assumed covered.
- API-key-class providers (no `CLAUDE_CONFIG_DIR`-equivalent concept) are
  out of scope — `resolve_bound_oauth_config_dir` only ever matches
  OAuth-class bindings, same restriction the original watcher path had.

## 4. Testing

- `identity/resolver/inject.rs`: new unit tests for
  `resolve_bound_oauth_config_dir` — resolves the bound dir for an
  OAuth-class identity-bound agent; returns `None` for an unbound/ambient
  agent (no regression to the fallback path); returns `None` when the
  agent's provider is API-key-class; returns `None` when no instance
  exists for the block; does not mutate account status or publish any
  broker event as a side effect (asserted via a broker handle that panics
  if `publish` is called — proves the "no side effects" claim, not just
  states it).
- `subagent_watcher/parse.rs`: `resolve_claude_config_dir` tests extended
  with the new parameter — `Some(dir)` wins over `cmd:env`; `None` falls
  through to existing behavior unchanged (regression coverage for the
  ambient-agent case).
