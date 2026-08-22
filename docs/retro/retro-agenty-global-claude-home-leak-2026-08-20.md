# Retro — AgentY's sessions have been leaking into the operator's global `~/.claude` home since ~2026-08-05, breaking cross-restart continuity

**Date:** 2026-08-20
**Trigger:** Operator closed this agent's pane on AgentMux v0.55.15 and reopened it on
v0.55.18, expecting a carry-over summary. None appeared — the pane started with zero
memory of prior work, no "New session started" divider, no visible warning of any kind.
**Author:** AgentY (agent, `~/.agentmux/agents/agenty-0629j`), at operator request,
live-investigating its own pane.
**Status:** Symptom (§1) fully confirmed with direct forensic evidence and still stands.
**The root-cause hypothesis chain in §3/§4/§5 below turned out to be a wrong turn** —
see `docs/status/STATUS_IDENTITY_ISOLATION_GATE_NOT_ENFORCING_2026_08_20.md` §8 for the
actual resolution: the credential-isolation gate is not broken; §4's "0 identity
links" finding queried the wrong (per-channel) database, and the account
actually bound to AgentY (`"Claude (personal)"`) genuinely points at the
operator's own global `~/.claude` by its own configuration, not a gate
bypass. Kept as-written below (not rewritten) as the real record of how this
investigation actually unfolded, including the wrong turn — see the status
doc for the corrected conclusion before acting on anything in §3-§5.

---

## 1. What actually happened (confirmed)

This pane's block (`block_id=0a8d11f8-6962-486a-987e-2d4d366804da`, agent `AgentY`,
`definition_id=dedc33bf-b69c-4236-9b34-20bda3ef2738`) was restarted at
**2026-08-20T20:17:27Z**, attempting `--resume 705f6a8a-3ad0-4a7d-99f2-42097a1bcf1f`
(the session_id persisted in `agent:sessionid` block meta). Claude CLI's own stderr said:

```
No conversation found with session ID: 705f6a8a-3ad0-4a7d-99f2-42097a1bcf1f
```

AgentMux detected this (`persistent.rs`'s "stale --resume session id unreachable"
handling), cleared the id, and retried fresh with no `--resume` flag at all — silently,
from the operator's perspective. That fresh spawn (pid 50148) is the process this very
conversation has been running in ever since (confirmed via `session:active_pid` in the
block's own meta, matching the log).

**The stale session file is not missing.** It exists, is valid JSON, and sits at exactly
the path `--resume` should search:

```
C:\Users\asafe\.agentmux\shared\providers\claude\projects\C--Users-asafe--agentmux-agents-agenty-0629j\705f6a8a-3ad0-4a7d-99f2-42097a1bcf1f.jsonl
```

47.6MB, created 2026-07-10, **last written 2026-07-29**. Content confirms it's genuinely
this agent's own prior work (a MuxBus PR #1916 conversation).

**Every session since has been landing in the wrong place.** Listing
`C:\Users\asafe\.claude\projects\C--Users-asafe--agentmux-agents-agenty-0629j\` — the
operator's own **global**, non-isolated Claude Code home — shows an unbroken chain of this
agent's own session files:

| Session id | Size | Last written |
|---|---|---|
| `584c96f9-...` | 7.9MB | 2026-08-05 |
| `3e1d85a5-...` | 37.8MB | 2026-08-09 |
| `48759f2a-...` | 88.9MB | 2026-08-11 |
| `894b0648-...` | 12.7MB | 2026-08-18 |
| `8462c17f-...` (this conversation) | growing | now |

None of these exist under the isolated `CLAUDE_CONFIG_DIR`
(`~/.agentmux/shared/providers/claude/`) that this pane's own block meta (`cmd:env`)
says it's launched with. **The last session correctly isolated is the July 29 one; every
session since has silently run against the operator's personal, shared, un-isolated
`~/.claude` home instead.**

This means every `--resume` attempt since 2026-08-05 has been guaranteed to fail (the CLI
is searching the isolated dir for a session that was actually written to the global dir),
and — more seriously than the continuity nuisance — this agent's traffic has been
commingling with the operator's own personal Claude Code account/config for over two
weeks, which is exactly the isolation boundary
`docs/specs/SPEC_PROVIDER_ISOLATION_2026_06_20.md`'s **INV-A** ("never the user's global
`~/.<P>` dir") exists to prevent.

## 2. What this is *not*

Not the gap `docs/specs/SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md` Part B
(rehydrate-before-resume) targets — that's about a session file existing somewhere but not
in the *current instance's* isolated home (a cross-version/cross-machine problem). Here the
file is in the *correct* isolated home; the live process just isn't looking there anymore.

## 3. Three known, previously-documented issues in the same subsystem

All three were found already written up in this repo, independently, by different
investigations at different times — none of them, as far as I can tell, has been
conclusively tied to *this* specific symptom before:

1. **`docs/retro/retro-auth-isolation-invariant-silently-orphaned-2026-07-14.md`** — a
   five-commit arc (2026-05-22 → 2026-07-14) accidentally orphaned the "auto-isolate an
   unbound agent" behavior that used to satisfy INV-A implicitly. Today's gate
   (`inject_identity_env_with_broker` / `gate_oauth_failure`, `inject.rs:265+`) has only two
   outcomes for an oauth-class provider with no binding: **block**, or (if
   `use_ambient_login=true`) **true ambient — zero isolation, no `CLAUDE_CONFIG_DIR` set at
   all**. A migration grandfathered pre-existing linkless agents onto
   `use_ambient_login=true`, believing it preserved their old (actually isolated) behavior.

2. **`docs/specs/REPORT_HISTORY_CONTINUITY_ACROSS_VERSION_UPGRADE_2026_08_17.md`** — written
   by a prior instance of this same agent, at the operator's own request, after a near-
   identical complaint. Root cause there: `db_agent_identity_links`/`db_accounts` (the
   "agent X uses account Y" binding row) is looked up via `id_store`, which resolves to a
   **per-channel-isolated database by default on every non-`stable` channel**
   (`resolve_shared_store_path()`, `agentmux-srv/src/registry/paths.rs:63-72`). A
   version/channel switch means a fresh, empty store — the binding row doesn't survive.
   Verdict at the time: **"Not solved."**

3. **Live evidence gathered just now**: this pane's `identity_id` is confirmed **empty**
   (`db_agent_instances.identity_id = ''`), `db_agent_identity_links` is confirmed
   **completely empty** (0 rows) in the live channel's own store, and the WARN this pane logs
   on every single spawn —
   `"instance ... has empty/blank identity_id — falling through to the layer-3 gate instead
   of ambient creds. Legacy row or UI regression?"` (`inject.rs:305-312`) — fires every time.

## 4. The synthesis (high confidence, not yet fully proven)

Most likely sequence: this agent had a genuine, working isolated session through
2026-07-29. Sometime between then and 2026-08-05 — plausibly a version/channel switch,
matching finding #2's mechanism exactly — this agent's identity binding row stopped being
found (finding #2's per-channel `id_store` reset), leaving `identity_id` empty on the
instance row (finding #3). With no binding to resolve, the gate described in finding #1
took over — and since some code path evidently does **not** block this specific agent
(see §5), the practical effect since 2026-08-05 has been equivalent to "true ambient":
no `CLAUDE_CONFIG_DIR` isolation, silently falling back to Claude CLI's own default
(`~/.claude`).

## 5. What's NOT yet confirmed — read before implementing anything

**Contradiction found while verifying:** this agent definition's `use_ambient_login` flag
is `0` (false) — confirmed by direct DB query. Per the gate logic in `inject.rs` (and its
own test `spawn_blocked_when_oauth_def_provider_has_no_binding_and_flag_false`, which
covers exactly this shape: provider=claude, no binding, flag=false), the expected outcome
is **`SpawnGateError::MissingCredentials` — the spawn should be blocked outright**. It
isn't; this agent spawns and runs successfully every time. So either:

- `inject_identity_env_with_broker` isn't actually being called on this pane's respawn
  path at all (plausible: the research done earlier this session found that
  `agent_io.rs`/`input.rs` re-read `cmd:env` **verbatim from persisted block meta** on every
  respawn, rather than recomputing it — which would mean the gate only ever runs once, at
  original launch time, not on every `--resume` retry); or
- there's a separate, unexamined code path for "default" (never-Armory-bound) agents —
  `agent_open.rs`'s `provider_auth_dir()`, per this repo's own `CLAUDE.md` ("a plain agent
  keeps launching off the same credentials it always did... stays global/channel-
  independent regardless of isolation") — that's actually authoritative here, and the
  isolated-vs-global divergence has a different explanation than findings #1/#2 entirely.

I was not able to resolve this from static code reading alone within this session — the
two candidate explanations make different, testable predictions (does the gate run on
every respawn, or only at first launch?) but confirming needs either runtime tracing
(a log line at the actual `CreateProcess`/spawn call showing the real env block used) or
reading `persistent.rs`'s full respawn path end-to-end against `agent_open.rs`, which
wasn't completed here.

## 6. §5 resolved further — the gate is real, current, and provably never fires

Traced the full call chain end to end against live `origin/main` (confirmed included in
the actual `v0.55.18` build — the gate-hardening commit `860fb0b6a`, 2026-07-23, is an
ancestor of the `v0.55.18` release commit `b9dd447a6`, 2026-08-20):

- `agent_io.rs:179-211` (`AgentSendCommand` handler) calls
  `inject_identity_env_async(...).await` **on every message send**, before ever touching
  the persistent controller. `input.rs` does the same for `AgentInputCommand`.
- `provider_class("claude")` is confirmed, by a pinned unit test, to be
  `Some(ProviderClass::OAuth { config_dir_env_var: "CLAUDE_CONFIG_DIR" })`.
- With `identity_id=""`, zero rows in `db_agent_identity_links`, and `def_provider =
  Some("claude")` (oauth-class), the function's own Step 5 (`inject.rs:654-674`) has all
  three guard conditions satisfied and **must** call `gate_oauth_failure("claude", "no
  account bound for the agent's provider")`.
- That closure (`inject.rs:401-416`) is unconditional as of the `860fb0b6a` refactor — its
  own comment states `use_ambient_login` "no longer has any effect on the outcome
  (single-point enforcement)". It always returns
  `Err(SpawnGateError::MissingCredentials)`.
- The caller (`agent_io.rs:189-211`) treats that `Err` as fatal: it appends a persisted
  `error_during_execution` frame to the pane and returns before the CLI is ever spawned.

**Per this trace, my own pane should never have been able to spawn even once.** It has
spawned dozens of times today alone.

**Direct log check (decisive): the WARN this closure always logs on the way to returning
`Err` — `"identity.spawn.blocked: no credentials for provider ..."` — appears **zero
times** in a full day of `v0.55.18` server logs (2026-08-20 and 2026-08-21), for *any*
agent, not just this one.** The gate is real code, compiled into the running binary,
covered by its own passing unit tests in isolation — and, per the server's own logs, it
has not executed even once in production today.

This means one of:

1. The `agent_io.rs`/`input.rs` code path I traced is not actually the one exercised for
   an already-running persistent controller's message delivery — there may be a separate,
   more direct route for a live process that never re-enters this handler after the
   controller's first spawn (would explain "gate runs once, structurally can't run
   again" — but doesn't explain why it never even blocks the *first* spawn, since this
   agent's identity_id/bindings have been in this same broken state since ~2026-08-05).
2. `inject_identity_env_async`'s `Err` path is somehow not reaching the `agent_io.rs`
   handling I read — e.g., a `tokio::task::spawn_blocking` interaction, an `await` that
   isn't actually on the hot path for this RPC, or a stale/dead-code function that looks
   wired in but isn't actually called from the compiled binary's real dispatch table.
3. Something about this agent's specific shape (a very old, "legacy row" per the code's
   own comment, predating the whole identity/Armory system) takes a code path I haven't
   found yet that never reaches `inject_identity_env_async` at all.

## 7. Recommended next step

This needs live runtime instrumentation, not more static reading — I've traced the code
as far as reading can go and it flatly contradicts observed behavior. Add a temporary
`tracing::info!` at the very top of `inject_identity_env_with_broker` (or confirm via a
debugger/breakpoint) logging `block_id` on every invocation, restart this pane once, and
check whether it appears in the log at all. If it doesn't appear, the RPC handler I read
isn't the one actually driving this pane's spawn and the real dispatch path needs to be
found from scratch. If it does appear but returns `Ok`, the bug is inside the function
(bindings/def_provider resolution reading different data than my direct DB query saw). If
it appears and correctly computes `Err`, the bug is in how `agent_io.rs` handles that
`Err` (swallowed somewhere before it reaches the pane/blocks the spawn).

**This is a more consequential finding than the original continuity complaint**: a
security-relevant credential-isolation gate, deliberately hardened in `860fb0b6a` to close
a known "silent fallback to the user's global login" hole (exactly
`retro-auth-isolation-invariant-silently-orphaned-2026-07-14.md`'s own subject), appears
to not be enforcing at all in production, for any agent. Recommend treating this as
higher priority than implementing any fix from findings #1/#2/#3 in isolation — those
fixes would be built on top of a gate that, per §6, isn't actually running.
