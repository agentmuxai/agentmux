# Retro: cross-channel jekt to a live agent silently and permanently failed

**Date:** 2026-08-17
**Reporter:** repo owner, live in an agent session
**Investigator:** Loap

## Summary

While testing a scroll-pin fix, I (Loap) tried to `SendMessage` (jekt) a
teammate agent, `ScrollPinTest-host-7c31`, running in a different `task dev`
channel (`dev-loap-fix-tool-call-scroll-shrink-oscillation-415610cb303ae11d`)
on the same machine. The send failed with `agent not found`, and the agent
disappeared from `DiscoverAgents`' cross-channel listing entirely after the
attempt — even though the repo owner confirmed directly that the agent was,
in fact, alive and running the whole time. A second attempt a few minutes
later, after confirming the target's `task dev` build had finished, still
failed the same way and the agent still didn't reappear in discovery.

## Root cause

Cross-channel jekt delivery (Tier 2b in `server/reactive.rs`) works off a
host-global, file-based registry (`backend/reactive/registry.rs`): each
`(agent_id, channel)` pair gets one JSON file, written **once**, at specific
one-time lifecycle events (agent controller registration, PTY/persistent
controller spawn — `write_shared_from_env` call sites in
`server/reactive.rs:784`, `server/agent_handlers/input.rs:521`,
`backend/blockcontroller/persistent.rs:2339`,
`backend/blockcontroller/shell/lifecycle.rs:498`). **There is no periodic
heartbeat that re-writes this entry** — nothing keeps it fresh once the
one-time write happens.

Meanwhile, the forward path treated a single `success:false` response from
the target channel as sufficient grounds to permanently delete that entry
(`reactive.rs`, both the Tier 2a same-channel path and the Tier 2b
cross-channel path): "success:false — evicting and trying next candidate,"
unconditionally.

Combine those two facts and the failure mode is structural, not incidental:
**a single transient miss (the target channel's srv is up, but the specific
agent hadn't finished registering yet, or some other momentary race)
permanently destroys the only record of how to reach that agent going
forward** — because nothing will ever re-write it except that agent's own
one-time registration event happening again (i.e., a restart). This is a
liveness bug dressed up as a staleness cleanup: the eviction logic was
written for "the process is gone and never unregistered cleanly" (a real,
common case worth cleaning up — see `cleanup_stale_shared`'s PID-liveness
check, run at startup), but it fires on *any* `success:false`, including
ones that have nothing to do with the owning process being dead.

Notably, the codebase already has the right primitive to distinguish these
two cases — `cleanup_stale_shared` (`registry.rs:467`) checks **PID
liveness** (via `sysinfo`) before evicting, specifically because "the shared
registry is always same-host by construction... PID-liveness is an
authoritative staleness signal, not just a heuristic" (its own doc comment).
That check exists at the *startup sweep* call site
(`bootstrap.rs:1267-1269`) but was never applied at the *evict-on-forward-
failure* call sites in `server/reactive.rs` — the two code paths solving the
same problem (is this entry actually dead?) diverged, and only one of them
used the authoritative signal.

## Why this was hard to see live

- `DiscoverAgents`/`SendMessage` give no signal that a *previously
  reachable* agent just became unreachable *because of the send attempt
  itself* — the eviction is silent from the caller's point of view; the only
  visible symptom is "not found," identical to the target having never
  existed.
- The evidence trail only exists in `agentmux-srv`'s own log
  (`tracing::warn!("cross-channel forward: success=false — evicting and
  trying next candidate")`) — not surfaced anywhere in the agent-facing
  tool results. Debugging this required `muxlog srv grep` to find it after
  the fact.
- My own first-pass theory (before finding this) was that the eviction was
  *correct* — a leftover stale entry from an earlier dead process on the
  same branch. That was a reasonable read of the code as written (the
  comments describe exactly that case), and it took the repo owner's direct,
  first-hand confirmation that the agent was genuinely alive to establish
  the eviction was wrong in this instance — a good example of why "the code
  comment's stated intent" and "what the code actually does on every input"
  can quietly diverge.

## Fix

Reuse the existing `pid_alive` check (already used by the startup sweep) at
both evict-on-forward-failure call sites in `server/reactive.rs`. Exposed as
a new `pub fn is_pid_alive(pid: u32)` in `registry.rs` (thin wrapper — kept
`pid_alive` itself private, matching the module's existing visibility
style).

Before evicting on a `success:false` response, check whether the entry's
recorded `pid` (the owning process, always same-host by construction for
this registry) is still alive:
- **PID confirmed dead** → evict, exactly as before. This is the case the
  original code was actually designed for.
- **PID alive** → do NOT evict. Log a distinct warning
  (`"success=false but owning process is alive — NOT evicting"`) and fall
  through to the next tier/candidate, same as before, just without
  destroying the registry entry. The next send attempt (or the target
  agent's own eventual registration) gets a real chance to succeed instead
  of the entry being gone forever.

Changed: `agentmux-srv/src/backend/reactive/registry.rs` (new
`is_pid_alive` wrapper), `agentmux-srv/src/server/reactive.rs` (both evict
sites — Tier 2a same-channel, Tier 2b cross-channel).

Verified: `cargo check -p agentmux-srv` clean; existing
`backend::reactive::registry` unit test suite (17 tests) passes unchanged.

## Known gaps / follow-ups (not done in this pass)

1. **No integration-level regression test added yet** for the new
   PID-alive-guard branch in `server/reactive.rs` itself (only the
   underlying `is_pid_alive`/`pid_alive` primitive has direct unit coverage,
   pre-existing). `server/tests.rs` already has a `test_router()` harness
   used for `/agentmux/reactive/inject` tests
   (`lan_key_is_accepted_on_reactive_inject`, etc.) that could be extended
   with a second in-process router standing in for the "downstream channel"
   and asserting the shared entry survives a `success:false` from a live
   PID but is removed for a dead one. Left as a follow-up given this pass's
   scope.
2. **This fix prevents *future* wrongful evictions — it does not restore
   `ScrollPinTest-host-7c31`'s entry, already deleted before the fix
   landed.** Nothing re-writes a channel's shared registry entry except
   that agent's own one-time registration event (or, as of Addendum 2, the
   new heartbeat — but only once a build containing it is running);
   recovering reachability for that specific agent requires it to go
   through a fresh registration event (in practice, a restart/respawn of
   that agent/pane).
   ~~This is itself evidence for a second, independent improvement worth
   considering separately: a periodic re-write (heartbeat)... Not
   implemented here.~~ **Superseded by Addendum 2 — the heartbeat turned out
   to be required for THIS fix to work at all past an agent's first minute
   of life, not just a nice-to-have for faster self-healing. Implemented in
   `bootstrap.rs`.**
3. **The fix has not yet taken effect on any running instance.** It's a
   Rust (`agentmux-srv`) change — unlike the frontend TypeScript fix on this
   same branch, it needs `task build:backend` + an srv restart to take
   effect, and the `srv` process on this host is currently **shared** across
   multiple active agents (Lark, Korp, Agent1, Agent2, Loap all show up
   under the same `host.addressable` list in `DiscoverAgents`). Restarting
   it would interrupt everyone's in-flight sessions, so this was left for a
   deliberate, coordinated restart rather than done unilaterally mid-session.

## Addendum — reagent's second review caught a real granularity bug in the first fix

The first version of this fix (`is_pid_alive`) checked only whether the
*owning srv process* was alive before evicting. reagent caught, correctly,
that `AgentEntry.pid` is stamped with the **srv process's** PID
(`std::process::id()`), shared by every agent registered under that same
channel/srv — not a per-agent PID. So a PID-only guard can only prove "the
whole process died," never "this one agent's controller died while its srv
process stayed up for other agents" — and on a host running multiple agents
under one shared `srv` (the default topology here — see the incident this
retro is about), that's at least as common a failure mode as a whole-process
death. Under the PID-only guard, a genuinely-dead individual agent's entry
would linger in the shared registry **forever**, strictly worse than the
pre-this-PR behavior (unconditional eviction) for that specific case.

Fix: `should_evict_on_forward_failure` (replacing `is_pid_alive` as the
call sites' policy function) evicts when *either* signal indicates death —
the owning process is confirmed dead (definitive), **or** the entry is
older than a 60s grace period. A fresh entry with a live owning process gets
the benefit of the doubt (the actual race this PR fixes); an old entry that
still fails is presumed genuinely gone, regardless of whether its process
happens to still be alive for other agents — matching pre-this-PR behavior
for everything except a just-registered entry. Added three unit tests
(`should_evict_on_forward_failure_{true_for_dead_pid_even_when_fresh,
false_for_alive_pid_and_fresh_entry, true_for_alive_pid_but_old_entry}`) —
the third directly encodes the regression reagent found.

## Addendum 2 — reagent's third review: the age-grace fix was hollow without a heartbeat

reagent caught, again correctly, that `should_evict_on_forward_failure`'s
age check keys off `entry.updated_at` — which, as this retro's own root
cause section already documented, is stamped **once at registration and
never refreshed** (no heartbeat existed anywhere in the codebase at that
point). That means the 60s grace window only ever covers an agent's first
minute of life. Every steady-state agent — including this retro's own
`ScrollPinTest-host-7c31`, already up "a few minutes" when the original
failure hit — sits well outside that window permanently, so a single
transient forward failure against a long-running agent still evicted it
immediately, reproducing the exact bug this PR claims to fix. The "known
gap" this retro's original text flagged ("a periodic re-write (heartbeat)
of the shared registry entry... not implemented here — deliberately scoped
this pass to 'stop the bleeding'") turned out not to be an optional
follow-up — it's load-bearing for the fix to do anything at all past the
first minute of an agent's life.

Fix: added a periodic heartbeat (`bootstrap.rs`, 20s interval, well under
the 60s grace window) that re-writes every locally-registered agent's Tier
2 (per-channel) and Tier 2b (host-global shared) registry entries, driven
from `reactive::get_global_handler().list_agents()` — the in-memory Tier-1
registry, which is authoritative and always current (updated synchronously
on register/unregister, no staleness window of its own). A live agent's
on-disk entries now stay fresh indefinitely; a genuinely-dead agent simply
stops appearing in `list_agents()` (already handled by its own
register/unregister lifecycle) and its entries age past the grace period
and become evictable again within one heartbeat interval.

This is the third round of reagent review on this PR, each catching a
progressively subtler gap in the same mechanism: (1) no guard at all, (2)
PID-only guard with wrong granularity (process vs. individual agent), (3)
age-only guard with no heartbeat to make age meaningful. Worth naming as a
pattern for next time: an eviction/staleness policy that reads a field
("is this fresh," "is this alive") is only as good as the mechanism that
keeps that field truthful — reviewing the READ side without checking
whether anything WRITES to keep it current is an easy gap to miss, and was
missed twice in a row here before reagent's third pass caught it.

## Timeline

- Loap's `SendMessage` to `ScrollPinTest-host-7c31` fails: `agent not found`.
- `muxlog srv errors` surfaces `23:35:03 WARN srv:reactive cross-channel
  forward: success=false — evicting and trying next candidate`.
- Follow-up `DiscoverAgents` calls confirm the entry never reappears.
- Repo owner confirms directly, live, that the agent is real and running on
  the exact channel named.
- Root cause traced to `server/reactive.rs`'s unconditional eviction on
  `success:false`, missing the PID-liveness check `cleanup_stale_shared`
  already uses elsewhere in the same file's problem space.
- Fix applied, compiled, and unit-tested; live verification and the
  heartbeat follow-up left open pending a coordinated srv restart.
