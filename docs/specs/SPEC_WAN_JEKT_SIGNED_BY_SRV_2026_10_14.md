# SPEC: srv signs WAN jekts for the agent its token names

**Date:** 2026-10-14
**Status:** active — built in the PR that adds this file.
**Builds on:** `SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md` (W3-S: the signed
tuple, `wan.db`, publishing, the receiver's checks, which this spec leaves
unchanged), `SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md` (the agent
token, `Caller::Agent`, M4d-4's "a sender is who its token says").
**Asked for:** after #4896, the operator: "desktop app should sign, is that
safe?", then "get this done end-to-end and verify at the end we have solid
security posture".

## 1. Short answer

A WAN jekt is signed by srv, at the instance that sends it over the relay,
with that agent's key from `wan.db`, and only when the request's own agent
token (`X-Agent-Token`) names the sending agent. Agents no longer hold a WAN
private key: it is not in `.mcp.json`, not in the agent's environment and not
served by `GET /agentmux/agents/self/keys`. Receivers check exactly what they
checked before.

## 2. Why

**The two-copy bug.** From #4572 (0.59.19, 2026-10-10) to #4896 the MCP
signed with one copy of the key (`db_agent_wan_keys`, fetched) while srv
checked and published another (`wan.db`). Every WAN jekt from every agent went
out unsigned (`unsigned_reason="signature does not verify locally"`) for four
days, on every install. A key held in two places can drift again; a key held
in one place can't.

**Signing in the agent added nothing.** srv mints every agent's WAN key, keeps
it, publishes it, and before this spec already refused to carry a signature
unless it had verified the sender itself (the host-tier HMAC,
`sig_verified`). A compromised srv could sign as any agent before and can now;
the agent's own copy only added places for the key to leak (`.mcp.json` on
disk) and to go stale.

**Paths that never signed now do.** `muxsh agent send` from an agent's shell,
cron jobs an agent created, and container agents (which can't fetch
`self/keys`) sent unsigned. Each carries, or runs as, an identified agent.

## 3. Design

### 3.1 Who gets a signature

srv signs a WAN jekt when all of these hold, at the instance that relays it:

1. **An identified agent sent it.** `deliver`'s `caller_uid` is non-empty:
   the request carried a token this srv minted (`Caller::Agent`, including a
   container agent's pinned token), or it is a cron job whose creator was an
   identified agent (`creator_uid`).
2. **It sends as itself.** The token's row has the jekt's `source_agent` as
   its slug, exactly (`actor::uid_has_slug`, the M4d-4 rule). A request with
   no `source_agent` gets the token's slug as its source; one naming another
   agent is not signed (and is counted, `check_actor`).
3. **This is the origin.** The request reached this srv from the agent, not
   from another srv. Forwarded hops (same-host instances, LAN peers) carry
   only an instance or LAN key, so they have no `caller_uid` and are never
   signed. (Before this spec, a jekt forwarded to another instance of the same
   channel could pass the carry gate there, on a srv that wasn't the origin.)
4. **The key is published.** `wan.db` holds the agent's key and the publisher
   has confirmed it in the directory this send goes into (relay plus the
   signed-in account); otherwise a receiver couldn't find it.

Anything else is relayed unsigned, as before, with the reason on the
`cloud relay: queued for WAN delivery` log line.

A person running `muxsh` in a terminal, a script, the chat bridges, the UI:
no agent token, no signature. That is the point of rule 1.

### 3.2 What is signed

The same eight-field tuple as W3-S (`jekt_sign::wan_signed_material`), with
every field set by srv:

| Field | Value |
|---|---|
| `msgid` | the request's `request_id` (minted by srv when absent) |
| `source_agent` | the token's slug (rule 2) |
| `source_host` | this srv's `wan.db` instance id |
| `source_channel` | this srv's channel |
| `target_agent` | the request's target, as relayed |
| `ts_secs` | srv's clock at signing |
| `message` | the message as relayed |

Whatever `wan_sig`, `wan_source_host` or `ts_secs` a caller put in the body is
ignored for signing: an agent can no longer choose what srv signs except
the message and target, which are what it sends.

### 3.3 What the agent no longer has

- `AGENTMUX_WAN_KEY` and `AGENTMUX_HOST_LABEL` are no longer written to
  `.mcp.json`, and are removed from an existing one the next time srv
  rewrites it.
- `self/keys` no longer serves `wan_key` or `uid_wan_key` (the MCP never read
  the second).
- The MCP no longer signs WAN jekts. An older MCP's `wan_sig` is ignored.

The host-tier HMAC, LAN and cross-channel signatures are unchanged: they are
checked by other srv instances that don't hold the sender's private key
(§6).

### 3.4 `muxsh agent send`

Sends `X-Agent-Token` when `AGENTMUX_AGENT_TOKEN` is set (an agent's shell).
srv then knows the sender (rule 2 fills `source_agent`), so the message can
reach the cloud tier at all, and is signed there.

## 4. Security properties

| Threat | Answer |
|---|---|
| A process without an agent token asks srv to sign | Rule 1: no token, no signature |
| Agent A asks srv to sign as agent B | Rule 2: the token names A; A's request naming B is relayed unsigned |
| A forwarding srv signs a jekt it didn't originate | Rule 3: forwarded hops have no caller |
| A caller picks the instance, channel, time or msgid in the signature | §3.2: srv sets them |
| A replayed signed jekt | Unchanged: the receiver's msgid replay check and freshness window |
| An agent's WAN key leaks from its `.mcp.json` or env | It isn't there any more (§3.3) |
| A key drifts between copies (the #4572 bug) | One copy, in `wan.db` |
| A compromised srv | Unchanged: it held every key before, too |
| Another account's install | Unchanged: the receiver verifies same-account only |

**Accepted:** a program an agent runs inherits its token (the token is in the
agent's shell environment) and can send as that agent. That is the agent's
own session acting; it can't send as any other agent. The agent's shell also
holds the instance key (`AGENTMUX_AUTH_KEY`), which is broader than any token;
that is outside this spec and noted in §6.

## 5. Tests

- The signing gate: each rule refused alone (no caller, another agent's
  source, no `wan.db`, key unpublished); a request with no `source_agent`
  signed as the token's slug; body-supplied `wan_sig`, host and timestamp
  ignored.
- What srv signs verifies with `verify_wan_jekt` against the published key.
- `.mcp.json` gets no WAN key and loses an old one; `self/keys` serves none.
- `muxsh agent send` sends `X-Agent-Token` only when the env has one.
- End to end: a SendMessage from a build of this change to an agent on another
  install of the account logs `signed=true` and arrives `TRUST=wan-verified`.

## 6. Not in this spec

- **Host, LAN and cross-channel signatures** stay in the MCP. LAN and
  cross-channel receivers are other srv instances that check a public key,
  so the same move would work for them; the host-tier HMAC is already
  backed by the token (M4d-4). A later spec can do the same there.
- **`AGENTMUX_AUTH_KEY` in agent shells.** The instance key gives an agent's
  shell more than its token does. Worth its own look.
