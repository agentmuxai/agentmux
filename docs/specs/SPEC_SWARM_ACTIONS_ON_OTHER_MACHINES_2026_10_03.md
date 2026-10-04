# SPEC: Acting on agents of other machines from the Swarm: instance-signed messages and a remote stop

**Status:** proposed
**Date:** 2026-10-03
**Author:** korp
**Trigger:** Repo owner, 2026-10-03, answering the open decisions of `SPEC_SWARM_REMOTE_AGENTS_PLATFORM_TAG_AND_SELECTION_2026_10_03.md` §9 with *"use recommended settings"*. Items 5 and 6 there (a message to another machine's agent, and a stop of one) are this spec: that spec's §6 says Phase 4 is where the security design is and gets its own spec.

**Builds on:**
- `SPEC_SWARM_REMOTE_AGENTS_PLATFORM_TAG_AND_SELECTION_2026_10_03.md` §5, §6: agents of other machines are selectable and listed, and an action reports them as "can't ... no verified link to other machines yet".
- `SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md`: per-agent LAN keys, first-use pinning, and what the LAN key does and does not prove.
- `SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md`: the install keypair, `<host>~<id8>`, `approved|new|revoked`, the host-gated approval window, freshness and replay.
- `SPEC_SWARM_BROADCAST_AS_USER_MESSAGE_2026_10_01.md` §2.1: why nothing here may claim the user's authority.
- `SPEC_AGENT_SELF_QUIT_2026_09_24.md` §6.5: the 15 s override window a stop gives the target's owner.

**Coordination:** AgentX owns the LAN discovery and WAN signing code this extends. They get this spec before any code. The cloud half needs a contract in the cloud repository, which is outside this spec (§5).

## 1. Summary

| # | What | Needs | Section |
|---|---|---|---|
| 1 | The Swarm's request to another machine is **signed by the sending install**, and arrives as what is proven: a message from that install's Swarm. It never arrives as the user's own turn | one install identity, shared by LAN and cloud | §3 |
| 2 | **LAN:** an install is known by a self-certifying id, shown as `<host>~<id8>`, and the receiving user can approve it | an identity route, a pin table, an approval window | §4 |
| 3 | **Broadcast** to LAN agents, as a verified message from that install | §4.2 | §4 |
| 4 | **Stop** of LAN agents, only from an approved install, and by default only if the target's user lets it | §4.3 | §4 |
| 5 | **Cloud:** the same signed message through the same-account path | a cloud contract | §5 |

## 2. What exists, and what does not

Checked in the code and specs on `main` at `0c72a2cd1`, 2026-10-03.

- **LAN proves an instance holds a key, not who it is.** The `lan_key` is advertised in the mDNS TXT record, so anything on the LAN can read it and it names an instance, not a person. Per-agent LAN keys prove which *agent* signed a jekt, pinned on first sight; the public key is fetched from whichever peer claims to host the agent, so the first lookup is trust on first use.
- **There is no install identity on the LAN.** The cloud has one: each install mints an Ed25519 keypair, its id is the first 128 bits of the SHA-256 of the public key (base32, 26 chars), so nobody can sign for an id without its key. Receivers record each install as `new`, and the owner can mark one `approved` or `revoked` in a window agents can't reach. Messages carry a freshness window and replay is refused.
- **A same-machine stop already has the owner's override.** Another channel on this machine is asked in its own window, 15 s to keep the agent (`forward_stop_pending`). Nothing like it crosses a machine.
- **"The Swarm sent it" does not mean "a person clicked it".** Any process in an agent pane holds the key that opens the same WebSocket the Swarm uses, and can call `fleet.broadcast` and `fleet.bulk-stop` (broadcast spec §2.1, findings 7 and 8). A signature by the sending install therefore proves the install's srv sent it, never that a human did.

## 3. The principle

**What the receiver is told is exactly what was proven.** Proven: install X's srv signed this request. Not proven: that a person at X asked.

So a request from another machine:
- is **not** a `[BROADCAST:FROM=user ...]` turn. That header is the user channel, which no network path can reach;
- arrives as a message **from that install's Swarm**, labelled so, under the **existing jekt rules**: `FROM=swarm@<host>~<id8>`, `TRUST=lan-verified` or `wan-verified`, `INSTANCE=<host>~<id8>`, `INSTANCE_STATUS=approved|new|revoked`;
- gets **no authority beyond a verified agent jekt**. Any agent on X can already send a verified, signed jekt to the agents on Y, so a broadcast from X's Swarm adds a label and no capability.

A stop is the exception, because no jekt can stop an agent today. It is the only new power, so it gets the strictest gate (§4.3).

## 4. LAN

### 4.1 One install identity, for LAN and cloud

Reuse the install keypair `wan.db` already holds, rather than minting a second one. One install then has one id everywhere, which is also what lets the Swarm merge a machine seen over the LAN and the cloud (other-hosts spec §3.3, which already planned `install_id` in the LAN record).

- **Advertise:** a new `install_id` in the mDNS TXT record and the UDP identity reply, next to `channel` and `os`. (`instance_id` is already taken: it is the release, `v<version>`.) It is a hint: the key is what proves it.
- **Identity route:** `GET /agentmux/instance/identity`, behind the same LAN-or-full auth as the other LAN routes, returns `{install_id, public_key, host_hint}`. The receiver checks `install_id == base32(sha256(public_key)[..16])` and refuses a mismatch. `host_hint` is validated as `[a-z0-9.-]{1,48}` and rendered only as `<host_hint>~<first 8 of the id>`, never alone.
- **Pin:** the first key seen for an `install_id` is pinned. Because the id is the hash of the key, a different key for the same id can't exist; the pin only has to remember the id and the key it was fetched with. A peer that claims a hostname it doesn't own simply shows as a new install under its own id.
- **Status:** `new` on first sight, `approved` or `revoked` by the receiver's owner. The approval window is the host-gated one the WAN spec defines (`credential_broker` pattern: a host window, accepted only on the host-secret channel). It shows the full id and warns when a new install's host hint matches an approved one. Until the fix for the same-user residual in the WAN spec §2.6 ships, approval is shown but only protects against MCP tools, as that spec says.
- **Store:** the receiver's known installs live in the same `wan_known_instances` table (the cloud already records installs there), so an install approved once is approved on both routes.

### 4.2 Broadcast

A new route on the receiver, `POST /agentmux/instance/message`, behind the LAN-or-full auth gate. The body is `{target, body, msg_id, ts, recipients, install_id, sig}`, with

```
sig = Ed25519(install key,
  "amx-swarm-msg-v1" ␁ install_id ␁ lowercase(target) ␁ msg_id ␁ ts ␁ recipients ␁ sha256(body))
```

The sender's srv signs; no agent holds the install key. The receiver:
1. Looks up or fetches the identity (§4.1) and verifies `sig`. A signature that is present but doesn't verify is `Some(false)`: forced to `TIER=sensitive`, `ESCALATE=required`, like a failed LAN or WAN signature.
2. Checks freshness: `ts` no more than 300 s old and no more than 60 s ahead; outside that is unsigned, not forged (the WAN rule).
3. Refuses a replay: `(install_id, msg_id)` is kept in the replay table in `wan.db` until its window passes, and written after successful delivery.
4. Delivers it through the reactive handler as a jekt from `swarm@<host>~<id8>`, with the delivery tier taken from the credential that authenticated the request (LAN spec §3), never from the body.

Tier rules, the same as any verified jekt: clean content is `TIER=coord`; a declared-sensitive tier or a keyword match is `TIER=sensitive`. For a sensitive message:
- from an **approved** install, `ESCALATE=none`;
- from a **new** install, `ESCALATE=required`;
- from a **revoked** install, forced sensitive and `ESCALATE=required`.

An unapproved install can still broadcast ordinary coordination. That is no worse than today, where anyone holding the `lan_key` can inject an unsigned message at `TRUST=network-claimed`; a signed one adds the id and the status.

### 4.3 Stop

`POST /agentmux/instance/stop-request`, same envelope and signature (`amx-swarm-stop-v1`), plus the target's block or agent name. It reuses the override machinery of a same-machine stop (`sagas/pending_shutdown.rs`):

1. The request is accepted **only from an approved install**. A `new` or `revoked` install is refused with "not approved by <this machine> yet", and the sender reports that for the target.
2. The receiver opens the override window in its own window, naming the requester as `swarm@<host>~<id8>`, and returns the request id.
3. **The default is to keep.** Unlike a local stop, nothing happens unless the target's user accepts the stop in the window, or has pre-granted that install the right to stop without asking (below).
4. The sender follows the result through a status route (`kept`, `accepted`, `stopped`, `expired`) and reports it per target.

Why the default is keep: a stop is the one new power, and any process on the sending machine can make its srv sign one (§2). An approved install's compromised agent could otherwise stop every agent on every approved machine whose user isn't watching. With keep as the default, the worst it does is raise a window.

**Pre-grant.** When approving an install the owner may also check "may stop my agents without asking". Only that grant makes a stop default to proceed after the 15 s window, as a local stop does. It is per install, stored with the approval, and shown wherever the install is listed.

### 4.4 The sender

The Swarm resolves each remote target `(host_id, channel, agent)` to its peer on the backend; the renderer never holds an address or key (other-hosts spec §6). Per target, the result is one of: delivered; refused (`not approved`, `kept by the user`, `older build`, unreachable). It lands in the same `FleetActionResult` panel, named with its machine and platform as in the selection spec §5.3, in place of today's "no verified link yet".

## 5. Cloud

Same-account agents on other installs use the same signed message, sent through the account's existing delivery path and verified by the receiver against the install key the account's directory already holds (`TRUST=wan-verified`, the WAN spec §2.3). The envelope is §4.2's; the replay table and freshness window are the WAN ones.

This repository does the client half: building and signing the message, and verifying and rendering it on receipt. The cloud needs a message type and routes for it, which is a contract in the cloud repository and is not specified here. It also waits for the cloud tier of the Swarm (other-hosts spec Phase 3), which is what lists those agents at all.

**A cloud stop is not planned.** A stop needs the target's window to be open now and an answer back, which the cloud path's 30-minute store-and-forward doesn't give, and a stop that waits in a queue until someone opens the app is the wrong behaviour.

## 6. Changes to the trust rules

Both change what an agent is told, so both ship with real code and tests in the same PR as the verifier, and update `CLAUDE.md`'s jekt section there (the WAN spec §2.6 sets that precedent), not before:
1. **A new sender kind**, `FROM=swarm@<host>~<id8>`, and the marker fields above on the LAN tier (`TRUST=lan-verified` with `INSTANCE=` and `INSTANCE_STATUS=`).
2. **`ESCALATE=none` for a verified message from an `approved` LAN install** (§4.2). Today the no-stop list is host-, LAN-agent-, channel-, reagent- and WAN-verified. This is a policy change and needs the owner's explicit confirmation at that time (§9 item 2), as the 2026-08-17 and 2026-09-26 changes had.

## 7. Security properties and residuals

- **No forged id.** An id is the hash of its key, so a peer can't take another install's id. It can claim a hostname, which is shown only as part of `<host>~<id8>` beside the status.
- **No new capability for broadcast.** A verified message from an install is what any agent on it can already send. The label is honest about the source: an install's Swarm, not a user.
- **Stop is bounded by the target's user.** Approved installs only, an override window, and default keep unless pre-granted.
- **No replay.** Freshness plus a stored `(install_id, msg_id)`, written after delivery.
- **The LAN key still opens the door.** Anyone holding it can reach the routes, but can't produce a signature, so everything it can send stays at `network-claimed`.
- **Residual: first sight.** The first time an id is seen it is `new`; whether the person approving is looking at the right machine rests on comparing the id shown in the approval window with the one the other machine's Armory shows. That is the standard first-use residual, and the id is long and hash-derived so it can be compared.
- **Residual: a same-user process on the sending machine.** It can read the install key from disk, or call the fleet RPC (§2). This is the machine-compromise residual the WAN spec §4 already accepts. Stop's default-keep exists so it can do little with it.

## 8. Phases

| Phase | Scope | Needs | Verified on |
|---|---|---|---|
| **4a: install identity over the LAN** | `install_id` in TXT and UDP reply; the identity route; the pin and status store reusing `wan_known_instances`; the approval window; the marker plumbing and `FROM=swarm@...`. No action yet. | AgentX's LAN code; reuse of the WAN install key | narko and a second machine, approve and revoke |
| **4b: broadcast** | §4.2 route, signing, freshness, replay, tier rules; the Swarm's per-target results. | 4a | the same pair |
| **4c: stop** | §4.3: status route, override window with default keep, the pre-grant. | 4a, 4b | the same pair |
| **4d: cloud** | §5 client half. | a cloud contract; the cloud tier of the Swarm | two installs on one account |

## 9. Tests

- **Identity.** A key whose hash isn't the claimed id is refused; the same id with a different key can't be pinned; a hostile `host_hint` renders as `?`; `install_id` round-trips through TXT and UDP and an older peer without it still lists.
- **Signature.** Each field of the signed tuple changes the signature; a bad signature forces sensitive and `ESCALATE=required`; a stale `ts` is unsigned, not forged; a replay is `Some(false)`; the delivery tier comes from the credential, not the body.
- **Tier rules.** Approved, new and revoked, with a clean message and a sensitive one: `coord`; `ESCALATE=none`; `ESCALATE=required`; `ESCALATE=required`.
- **Stop.** A `new` or `revoked` install is refused; an approved one opens a window; with no answer a default-keep install's request expires and the agent keeps running; with the pre-grant it proceeds after 15 s; "keep" ends it; the sender sees each outcome per target.
- **Sender.** The renderer never receives an address or key; one unreachable peer fails only its own targets.
- **Safety.** No remote request is ever delivered with a `[BROADCAST:FROM=user` header or as `TurnOrigin::User`.

## 10. Decisions for the owner

Each has a proposed answer; "use recommended settings" accepts them.

1. **One install key for LAN and cloud** (proposed), or a separate LAN-only key.
2. **`ESCALATE=none` for a sensitive message from an approved LAN install** (proposed; a `CLAUDE.md` policy change, to be confirmed by you again when the PR ships).
3. **A `new` LAN install may broadcast ordinary messages** (proposed; no worse than the `lan_key` today), or approval is required first.
4. **A remote stop defaults to keep** unless the target's owner pre-granted that install (proposed), or it proceeds after 15 s like a local one.
5. **No cloud stop** (proposed).
6. **Order:** 4a, 4b, 4c on the LAN first, then 4d when the cloud contract and the Swarm's cloud tier exist (proposed).
