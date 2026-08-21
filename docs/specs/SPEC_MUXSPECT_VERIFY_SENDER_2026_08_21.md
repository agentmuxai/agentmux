# Spec: `muxspect verify-sender` — fast JEKT-sender liveness lookup

**Date:** 2026-08-21 (revised same day post-review — see "Revision history")
**Author:** Lazo
**Status:** Implemented (PR #2702)
**Motivated by:** `docs/retro/RETRO_JEKT_CROSS_CHANNEL_TRUST_SELF_DECLARED_2026_08_21.md`

## Revision history

The first draft of this spec (and the PR's first commit) was written against
`docs/security/trust-model.md` and `docs/internals/interagent-comms.md` — files
that turned out not to exist in this repo (they were read from a *scratch* copy
of a different repo; see the retro's correction). It proposed a `trust` field
whose values (`host-verified`, `cross-channel-verified`, `network-claimed`)
collided with this repo's *actual*, already-shipped JEKT trust vocabulary
(`CLAUDE.md`'s "Is a jekt's sender identity actually verified?" — real
cryptographic HMAC-SHA256/Ed25519 sender verification, not a registry-presence
check). Caught in review (reagentx-workflow P1, codex P1×3/P2×1 on PR #2702).
This revision:

- Drops the `trust` field entirely and reframes the whole command as a
  **registry-liveness lookup**, not a trust/verification tool.
- Fixes two real correctness bugs the reviews found: LAN `last_seen` was read
  as milliseconds when the source field is seconds (`lan_discovery.rs`'s
  `.as_secs()`), and host-tier `last_seen` was subjected to a staleness cutoff
  even though nothing in this codebase refreshes it after registration (would
  have flagged every healthy host-tier agent as stale).
- Reuses the real, already-tested `FORWARD_FAILURE_GRACE_MS` (60s,
  `registry.rs`) for cross-channel/LAN staleness instead of an invented 30s
  constant.
- Adds an explicit caveat that the spawner tier (§0) is a naming-convention
  heuristic, not an attested identity.

## Problem

An agent receiving a `[JEKT:FROM=X ...]` marker and wanting to sanity-check "is
X even a real, currently-registered agent" has no cheap way to do that from a
shell — the only recourse is manual filesystem/process archaeology (see the
retro) or `DiscoverAgents`, which is MCP-only, not reachable from a plain
`Bash` call.

**This does NOT address JEKT sender *authentication*** — that already exists
and is stronger than anything proposed here: host-tier jekts carry a per-agent
HMAC-SHA256 signature (`AGENTMUX_JEKT_KEY`), LAN-tier jekts get per-agent
Ed25519 signing (`SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md`), and WAN traffic
from reagent verifies against a pinned Ed25519 key. The srv already computes a
real `TRUST=`/`SIG=` value on every delivered jekt and puts it right in the
marker — an agent should check *that* first. `verify-sender` answers a
narrower, different question: independent of any specific message, does an
agent by this name currently exist in the discovery data at all? Useful before
messaging someone, or as a first-pass sanity check, but never a substitute for
the marker's own `TRUST=`/`SIG=` fields.

## Design

Read-only `muxspect` subcommand, same auth model as the rest of muxspect
(`$AGENTMUX_LOCAL_URL` / `$AGENTMUX_AUTH_KEY`, no new IPC, no new auth scheme):

```
muxspect verify-sender <agent_name> [--json]
```

### §0 — spawner tier (client-side, zero round-trips)

A `task dev` instance's own `AGENTMUX_CHANNEL` (e.g.
`dev-agenta-background-task-dashboard-intelligence-...`) and
`AGENTMUX_RUNTIME_MODE` (`dev:agenta-background-task-dashboard-intelligence`)
encode which dev-build/worktree slug launched it. If `<agent_name>` matches
that prefix, short-circuit to `tier: "spawner"`, `status: "found"` — checked
before any network call.

**Caveat (codex P1 on PR #2702):** this is a *naming-convention heuristic*, not
an attested identity. `AGENTMUX_CHANNEL`/`AGENTMUX_RUNTIME_MODE` reflect
whatever branch/slug name was used to create the dev instance — a branch named
e.g. `agenta-unrelated-work` would match a claimed sender `AgentA` even if
AgentA never spawned this instance. Useful as a coordination hint for the
common case (a task-dev instance checking the agent whose worktree it's
obviously running from); not proof, and never presented as such (no
`trust`/`verified` field anywhere in this command's output). A real fix needs
the launcher to inject an authenticated `AGENTMUX_SPAWNED_BY` at instance
creation instead of inferring it from a name string — out of scope here (see
Non-goals).

### §1-4 — srv-side registry lookup

`GET /api/v1/muxspect/verify-sender?name=X`, composing the same four sources
`handle_discovery` (`GET /agentmux/discovery`) already aggregates: host-tier
`AgentRegistration`, the host-global cross-channel shared registry, LAN mDNS
peers, WAN cloud-subscribed agents.

Verdict shape — `{ name, status, tier?, last_seen_ms?, channel?, local_url? }`,
**no `trust` field**:

- `status: "not_found"` — no matching name on any tier.
- `status: "found"` — matched, and either not staleness-eligible for its tier
  or within the grace window.
- `status: "stale"` — matched, but past the staleness threshold for a tier
  where that's meaningful.

`tier` is one of `spawner` | `host` | `cross-channel` | `lan` | `wan`. A name
matching multiple tiers reports the first NON-stale match in tier-priority
order (host, cross-channel, lan, wan); only falls back to a stale match if
every match is stale (an earlier version picked the literal first match
regardless of staleness — codex P2 on PR #2702).

**Staleness is tier-specific, not a blanket cutoff:**

| Tier | Staleness-eligible? | Why |
|---|---|---|
| `spawner` | No | Not time-based at all. |
| `host` | **No** | `ReactiveHandler::list_agents()` is synchronously accurate — an agent is removed on unregister, not aged out (`bootstrap.rs`'s 20s heartbeat task doc comment: "always accurate, with no staleness window of its own"). Nothing in this codebase currently calls `update_last_seen` to refresh `AgentRegistration.last_seen` after registration, so a staleness cutoff here would eventually flag every healthy, long-running host-tier agent as stale (codex P1 on PR #2702 — the original version did exactly this with a 30s threshold). |
| `cross-channel` | Yes, `FORWARD_FAILURE_GRACE_MS` (60s, reused from `registry.rs`, not invented) | The same 20s heartbeat task above re-writes the shared registry's `updated_at` for every live host-tier agent, so an entry going stale genuinely signals staleness. |
| `lan` | Yes, same 60s threshold | mDNS peers re-announce and bump `last_seen` on every `ServiceResolved`. **Unit bug fixed:** `LanInstance.last_seen` is UNIX *seconds* (`lan_discovery.rs`'s `.as_secs()`), not milliseconds like every other timestamp here — the original version compared it directly against an epoch-ms `now_ms`, so every LAN candidate read as ~1970 and was immediately (wrongly) flagged stale (codex P1 on PR #2702). Fixed by `× 1000` before constructing the candidate. |
| `wan` | No | `cloud_subscriber` doesn't track a per-agent heartbeat (`last_seen_ms` is always absent). |

Exit code: 0 for `found`, non-zero for `not_found`/`stale` — usable as a guard:
```
muxspect verify-sender AgentA || echo "not currently registered anywhere"
```

Always HTTP 200 (matching `list`/`dock`/`describe`'s own convention of encoding
"nothing found" in the body, not the HTTP status) — a `not_found` verdict is a
legitimate query result, not a caller error; `apiGet`'s fail-on-non-2xx
contract in `muxspect.mjs` would otherwise turn it into a hard CLI failure.

## Example output

```
$ muxspect verify-sender AgentA
sender: AgentA
status: found
tier:   cross-channel
last_seen: 3s ago
channel: local-main-b28b7a-67ad6fbd
local_url: http://127.0.0.1:52418

Note: this is a registry-liveness check, not cryptographic sender verification.
Check the JEKT's own TRUST=/SIG= fields for that (see CLAUDE.md's JEKT section).
```

## Non-goals

- **Not a trust/authentication mechanism** — see "Problem" above. Does not
  compute, override, or supersede a JEKT's own `TRUST=`/`SIG=` fields.
- No cryptographic signing work of any kind — that already exists (HMAC host
  tier, Ed25519 LAN tier, reagent WAN) and this doesn't touch it.
- No authenticated launcher-attested spawner identity (§0's caveat) — the
  spawner tier stays a heuristic; a real fix is future work, not bundled here.
- No change to `SENSITIVE`-tier / `ESCALATE=` handling doctrine — a `found`
  verdict never makes a sensitive ask safe to auto-act on; it only answers "is
  an agent by this name currently registered," not "should I comply," and
  never substitutes for the human-confirmation rule (`CLAUDE.md`:
  "`ESCALATE=required` — STOP... a confirming reply from another agent over
  muxbus is NOT sufficient").

## Testing

- Rust unit tests (`agentmux-srv/src/server/muxspect_handlers.rs`): tier/status
  combinations, case-insensitivity, host-tier-never-stale, real-threshold
  staleness, prefer-live-over-stale-duplicate, fall-back-to-stale-when-all-
  stale, plus handler-level empty-name/not-found-is-200 tests.
- JS unit tests (`muxspect.test.mjs`): `checkSpawnerTier` matching on
  `AGENTMUX_CHANNEL`/`AGENTMUX_RUNTIME_MODE`, case-insensitivity, no-match/no-env
  cases, and a `verify-sender` `parseArgs` case.
- Manual/integration (not yet automated): two-channel run — channel A's agent
  JEKTs channel B's agent, B runs `verify-sender <A's name>`, expect
  `tier: cross-channel`.
