# SPEC — honest account-delete semantics: spawn gating, agent reconciliation, Armory truthfulness

**Date:** 2026-07-14
**Status:** Approved (user decision on §2 recorded this date); implementation dispatched
**Governing analysis:** `docs/analysis/ANALYSIS_ACCOUNT_DELETE_AUTH_LIFECYCLE_GAP_2026_07_14.md`
(layers 2.1–2.4). Layer 1 (cascade + token-dir cleanup) shipped in PR #2159.
This spec covers the remaining layers 2 (running-agent reconciliation),
3 (ambient-fallback gating — the load-bearing one), and 4 (UI truthfulness).

---

## 1. Decision record

**User decision (2026-07-14): fail by default + explicit per-agent opt-in.**
When an oauth-class provider has no resolvable account for an agent at spawn
time, the spawn FAILS with a clear, user-visible error ("no credentials for
provider anthropic — bind an account in the Armory or enable 'use global
login' for this agent") — unless the agent carries an explicit
`use_ambient_login` (naming: see §3.2) opt-in, in which case the CLI's
default behavior (reading the user's global `~/.claude`) is allowed and
**surfaced**, never silent.

Rejected alternatives: hard-fail-always (breaks the casual personal-login
flow), keep-fallback-but-loud (delete still wouldn't deauthenticate a
respawned agent — fails the core requirement).

## 2. Layer 3 — spawn gating (backend, the load-bearing change)

### 2.1 Current behavior

`identity/resolver.rs` per-binding resolution treats every failure —
including "account row not found" (the post-delete case) — as log-and-skip
(~lines 577–590). With nothing injected, the spawned CLI silently uses the
ambient global login. No AgentMux log line even records that the fallback
happened (the CLI does it, not us).

### 2.2 New behavior

At spawn assembly, for each oauth-class provider the agent is *supposed* to
have credentials for (i.e. a binding/link exists OR the agent previously
launched with that provider):

- Account resolves → inject config-dir env var (unchanged).
- Account missing/unresolvable AND agent has `use_ambient_login = false`
  (default) → **spawn fails** before the CLI process is created. The error
  must reach the agent pane UI (the same surface other spawn failures use),
  wording: `no credentials for <provider>: the bound account was deleted or
  is unresolvable. Bind an account in the Armory, or enable "Use global
  login" in this agent's settings.` Log `tracing::warn!` with prefix
  `identity.spawn.blocked:` (extends the `muxlog auth` vocabulary — update
  the muxlog regex to cover `identity\.spawn`).
- Account missing AND `use_ambient_login = true` → proceed WITHOUT injecting
  a config dir (CLI uses its global default), and log
  `identity.spawn.ambient: using global CLI login (per-agent opt-in)` at
  info. This is the only sanctioned ambient path.
- Edge: agent has NO binding at all for the provider and never had one
  (fresh casual agent, never touched the Armory) — treat as
  `use_ambient_login` implicitly true? **No.** Implicit ambient is how we
  got here. Instead: the agent-creation flow keeps working because agent
  creation (launch modal) already binds-or-creates an identity; agents that
  genuinely have no identity record for the provider get the ambient path
  ONLY via the explicit flag. Migration (§2.4) grandfathers existing agents.

### 2.3 The opt-in flag

Per-agent, persisted on the agent definition (`db_agents` — follow the
existing pattern for agent-level booleans; check how e.g. model/effort
settings are stored). Exposed in the Agent setup modal (Accounts tab is the
natural home) as "Use global CLI login when no account is bound" with copy
that says what it means. RPC plumbing mirrors existing agent-settings
updates.

### 2.4 Migration / rollout safety

Existing agents that currently rely on ambient fallback must not all break
on upgrade: a migration sets `use_ambient_login = true` for agents that have
NO **oauth-class** identity link rows at migration time (they were de-facto
ambient users for their CLI login), and `false` for agents that have an
oauth-class link (they opted into managed CLI accounts; honest failure is
the correct new behavior for them). Api-key-class links (e.g. a github PAT)
do NOT count against grandfathering — they are never spawn-gated, so an
agent whose only link is a PAT was still an ambient CLI user, and blocking
it would break exactly the users this section exists to protect.
(Clarified 2026-07-14 during implementation review: the original wording
said "no link rows" of any kind, which contradicted this section's own
rationale.) This makes the honest semantics apply exactly where the user
expressed intent.

### 2.5 Tests (the resolver test forces the semantics)

- binding → missing account → flag false → spawn-assembly returns the
  blocking error (not a skip).
- binding → missing account → flag true → no injection + ambient log +
  spawn proceeds.
- resolvable account → unchanged injection (regression).
- migration test: linkless agent → flag true; linked agent → flag false.

## 3. Layer 2 — running-agent reconciliation (on delete)

Chosen shape: **(b) surface, don't hard-kill.** Deleting an account marks
affected running agents rather than terminating mid-turn work; combined with
layer 3, the next spawn/restart is where enforcement lands. (Hard-stop
remains available to the user per-pane as always.)

- On `deleteidentityaccount` (and `unlinkagentidentity`), after the cascade:
  look up affected agent ids (the links are being deleted in the same
  transaction — capture the agent_ids BEFORE the delete, inside
  `identity_delete`, and return them in `IdentityDeleteOutcome`).
- Publish a targeted event per affected agent (follow the existing
  `agentidentities:changed:<agent_id>` pattern) carrying
  `credentials_revoked: true` + provider.
- Agent pane subscribes and shows a persistent, dismissable state chip:
  "Credentials revoked — this agent still holds tokens until restarted."
  Wording must be honest: the process is still authenticated (analysis
  §2.1); we are disclosing, not pretending to revoke.
- Log `identity.delete: N running agent(s) affected` (info, auth vocabulary).

## 4. Layer 4 — Armory truthfulness

While any *running* agent was using a deleted account (layer 2's affected
set), the Accounts tab shows a transient notice row / toast: "Account
deleted. N running agent(s) still hold its tokens until restarted." No new
persistent state — the account row is gone (that's correct); this is a
disclosure at delete time, driven by the same `IdentityDeleteOutcome` data.
The delete confirmation dialog (if one exists; add one if not) shows the
affected-agent count BEFORE the delete: "2 running agents use this account."

## 5. Deferred (unchanged from the analysis)

- Provider-side token revocation (CLI `logout` subprocess) — follow-up.
- Hard-stop-on-delete option — revisit only if disclosure proves
  insufficient in practice.

## 6. Sequencing

Two PRs, parallel-safe:
- **PR A (layer 3):** resolver gating + flag + migration + modal toggle +
  muxlog regex extension + tests. Backend-heavy.
- **PR B (layers 2+4):** `IdentityDeleteOutcome.affected_agents` +
  events + pane chip + Armory delete-time disclosure + tests.
  Touches the delete handler that PR A doesn't.
Shared surface: `IdentityDeleteOutcome` is PR B's; PR A must not touch it.
