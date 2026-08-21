# Spec: `muxspect verify-sender` — fast JEKT sender verification

**Date:** 2026-08-21
**Author:** Lazo
**Status:** Proposed
**Motivated by:** `docs/retro/RETRO_JEKT_CROSS_CHANNEL_TRUST_SELF_DECLARED_2026_08_21.md`
**Addresses:** `SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01.md` §0.1 ("No trust
verification... An attacker who can inject text into an agent's input stream can
forge a jekt from a trusted peer") for the same-host case, without requiring the
§5.3 HMAC-signing work.

## Problem

An agent receiving a `[JEKT:FROM=... TRUST=...]` marker has no cheap way to check
whether `FROM` is a real, currently-live agent. The only recourse today is manual
filesystem/process archaeology (see retro) or trusting the `TRUST` field as-is —
which currently emits an undocumented `self-declared` value for cross-channel,
same-host delivery because no tier between `host-verified` and `network-claimed`
is defined for it.

The data to answer this already exists — `DiscoverAgents` (MCP) exposes
`host.addressable`, `host.cross_channel`, `lan`, and `wan.subscribed_agents` — but
it's an MCP tool, not available from a plain shell, and nothing correlates it back
to a specific JEKT's claimed sender in one step.

## Design

New read-only `muxspect` subcommand, same auth model as the rest of muxspect
(`$AGENTMUX_LOCAL_URL` / `$AGENTMUX_AUTH_KEY`, no new IPC, no new auth scheme):

```
muxspect verify-sender <agent_name> [--json]
```

1. New route `/api/v1/muxspect/verify-sender?name=<agent_name>` on the sidecar,
   backed by the same lookup `DiscoverAgents` already does across
   `host.addressable`, `host.cross_channel`, `lan`, `wan.subscribed_agents`.
2. Returns a verdict, not raw discovery data:
   - `not_found` — no matching agent on any tier. Exit code 1.
   - `found` — with `tier` (`host` | `cross-channel` | `lan` | `wan`),
     `last_seen_ms`, and a **computed** `trust` value derived from the tier
     (`host-verified` for `host`, `cross-channel-verified` for `cross-channel` —
     new value, same-machine but not in-process — `network-claimed` for `lan`/`wan`),
     independent of whatever the original JEKT's `TRUST` field said.
   - `stale` — found but `last_seen_ms` older than the existing 30s
     addressable-drop threshold (`interagent-comms.md`'s own heartbeat rule).
3. Exit code 0 for `found`, non-zero for `not_found`/`stale` — usable as a guard:
   ```
   muxspect verify-sender AgentA || echo "unverified sender, escalate to human"
   ```

## Example output

```
$ muxspect verify-sender AgentA
sender:      AgentA
status:      found
tier:        cross-channel
trust:       cross-channel-verified   (same machine, different AgentMux channel)
last_seen:   3s ago
channel:     local-main-b28b7a-67ad6fbd
local_url:   http://127.0.0.1:52418
```

## Secondary fix (backend, smaller)

Compute `TRUST` server-side at injection time from the actual delivery path
(`wrap_jekt_message`, per `jekt-visibility-completion.md`'s file list) instead of
ever emitting the `self-declared` placeholder. Cross-channel delivery already knows
it's 127.0.0.1-only same-machine (that's how `cross_channel` forwarding works at
all) — it can stamp `cross-channel-verified` directly instead of a value that reads
as "unverified" to both the receiving agent and any human watching the pane.

## Non-goals

- No cryptographic signing (§5.3 of the parent spec) — this only surfaces liveness
  + tier data the srv already computes for `DiscoverAgents`.
- No change to `SENSITIVE`-tier handling doctrine — `verify-sender` returning
  `found` still doesn't make a `sensitive`-tier ask safe to auto-act on; it only
  answers "is this sender real," not "should I comply."

## Testing

- Unit: route handler against fixtures for each of the four `DiscoverAgents`
  buckets, plus a stale-heartbeat case.
- Integration: two-channel test (mirrors `jekt-visibility-completion.md`'s own
  cross-channel test) — channel A's agent JEKTs channel B's agent, B runs
  `verify-sender <A's name>`, expect `tier: cross-channel`.
- CLI: argument parsing follows the existing `parseArgs` pattern in `muxspect.mjs`
  (positional name, `--json` flag) — add to `muxspect.test.mjs`.
