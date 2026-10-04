# SPEC: Swarm remote agents: a platform tag, and selecting them like local agents

**Status:** proposed
**Date:** 2026-10-03
**Author:** korp
**Trigger:** Repo owner, 2026-10-03: *"we want to refine how swarm works ... notice the new remote machines that appear. We want to be able to select them the same as the local swarm agents. We need to know the platform of the host (use a tag like MacOS, Linux, or Windows.) And we want to know if the agent is on same host, via lan, or via cloud. (there are already tags for that)"*

**Builds on:**
- `SPEC_SWARM_OTHER_HOSTS_AND_CHANNELS_2026_10_02.md`: the remote sections, the `this machine` / `LAN` / `cloud` tags, and the decision to keep remote rows read-only (§3.2, §7, §9 item 3), which this spec revisits.
- `SPEC_MULTI_AGENT_FLEET_CONTROL_2026_08_20.md`: selection, broadcast, bulk stop, saved groups.
- `SPEC_SWARM_BROADCAST_AS_USER_MESSAGE_2026_10_01.md`: what a Swarm broadcast is, and why it is not a jekt.
- `SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md` and `SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md`: what LAN and cloud traffic can and cannot prove about its sender.

**Coordination:** AgentX owns the multi-host Swarm (#4239, #4241). This spec adds a field to the files they own (`swarm_remote.rs`, `swarm-remote.ts`, the LAN advertisement in `lan_discovery.rs`) and a selection layer on top of their sections. It changes none of their merge, freshness or naming rules. They get this spec before any code is written.

## 1. Summary

| # | What | Needs | Section |
|---|---|---|---|
| 1 | Each remote section shows the host's **platform** next to how it was found: `[Windows] [LAN]` | one advertised field, `os` | §3 |
| 2 | Remote rows get the **same selection checkbox** as local rows, and a per-host "select all on this machine" | nothing on the wire | §4 |
| 3 | The actions on a selection say, per target, **which machine and how it was reached**, and do only what that route can do safely | per-tier delivery rules | §5 |
| 4 | Acting on agents of **other machines** (LAN, cloud) is **not** enabled by selection alone: it needs a sender identity that does not exist on the LAN today | owner decision | §6 |

The platform tag and the selection UI are safe to build now. Of the actions, two already work between channels on one machine and need only the UI. The rest wait on §6.

## 2. What exists

Checked in the code on `main` at `a5ed16855`, 2026-10-03.

- **The tags.** `swarm-remote.ts` labels each section `this machine`, `LAN` or `cloud` from the host's `tier`. There is no platform anywhere on the path: `LanInstance`, the mDNS TXT record (`version`, `hostname`, `instance_id`, `channel`, `auth_key`), the UDP identity reply, and the shared registry's `AgentEntry` carry no OS field.
- **Remote rows are read-only.** The row is a name in a `div`. There is no checkbox, and `Select all` takes only this instance's block ids (`allBlockIds`).
- **Selection is keyed by block id.** `selectedBlockIdsAtom` is a set of block ids, and `fleet.broadcast` and `fleet.bulk-stop` take block ids. A remote agent has a block id only when it is on this machine (the shared registry carries `block_id`). A LAN peer's agent list is names only.
- **Bulk stop already reaches other channels on this machine.** When a block id has no local controller, `fleet_bulk_stop_impl` forwards the stop over loopback to the channel that owns it (`forward_stop_to_shared_channel`, using that entry's `auth_key` from the shared registry, same user, same machine). Today the only gap is that the UI cannot select such an agent.
- **Broadcast does not.** `fleet.broadcast` resolves each target through this instance's own agent map (`get_agent_by_block`) and fails with "no registered agent for this block" for anything else. It delivers a `[BROADCAST:FROM=user VIA=swarm ...]` user turn, deliberately not a jekt (broadcast-as-user-message spec).
- **LAN identity is instance-level only.** The `lan_key` proves the caller holds a credential for that instance. That key is itself advertised in the mDNS TXT record, so anything on the LAN can read it. Per-agent Ed25519 keys prove which *agent* sent a jekt; nothing proves which *person* operates an instance.
- **Cloud has instance identity.** Every install has its own keypair and a same-account directory of other installs' keys (`TRUST=wan-verified`, `INSTANCE=<host>~<id8>`). The cloud tier of the Swarm (Phase 3 of the other-hosts spec) is not built.
- **The confirm list and the result panel show raw block ids.** The stop confirmation lists `{blockId}` and `FleetResultPanel` prints `{id}`. That is already poor for local agents, and unusable once two machines each have an agent called `Agent3`.

## 3. The platform tag

### 3.1 What it shows

A small tag on each remote section's header, before the existing how-found tag:

```
narko · stable   [Windows] [this machine]
Area54           [macOS]   [LAN]
home-desktop     [Linux]   [cloud]
```

- Labels: `Windows`, `macOS`, `Linux`. Any other value is shown as no tag, not guessed. A peer that does not advertise one (an older build) shows no platform tag. Nothing derives a platform from a hostname or a version.
- The tag is on the section, which is already one per host and channel. It is not repeated per row. It reappears wherever agents of several machines are listed together (§5.3).
- This machine's own header, if one is shown, gets the same tag.

### 3.2 Where the value comes from

One lowercase string, `std::env::consts::OS` (`windows`, `macos`, `linux`, ...), advertised as `os`:

| Tier | Source |
|---|---|
| This machine, other channels | this instance's own value. Every channel on a machine shares it, so the shared registry needs no new field. |
| LAN | a new `os` entry in the mDNS TXT record and in the UDP identity reply, next to `channel` (the same two places `channel` was added in #4241). Older parsers ignore an unknown field. |
| Cloud | a new `os` field in the signed install record that the cloud tier publishes (add to the record sketched in §5 of the other-hosts spec). |

It is per instance, so a Windows machine that also runs an instance inside WSL shows two sections: the Windows channel as `Windows`, the WSL one as `Linux`. They already appear as separate sections.

### 3.3 Trust

The platform is self-reported by the peer and is **display only**. No decision reads it. On the LAN anything can advertise anything, so the receiving side accepts a value only if it matches `^[a-z0-9_-]{1,16}$`, and the frontend maps only the three known values to a label. Everything else is dropped. The string never goes into a command, a path or an HTML attribute.

## 4. Selecting remote agents

### 4.1 A target is an address, not a block id

Selection today is a set of block ids. A remote agent needs an address that is stable across the 12 s refresh and unique across machines:

```ts
type FleetTarget =
    | { kind: "local"; block_id: string }
    | { kind: "remote"; host_id: string; channel: string; agent: string; block_id?: string };
```

The selection set holds a string key per target (the block id for local, `host_id/channel/agent` for remote). The frontend never holds a peer's address or key: the backend resolves `(host_id, channel, agent)` to the peer itself, as it does for the sections today. This also keeps the LAN key out of the renderer, which §6 of the other-hosts spec asks for.

### 4.2 The controls

- **A checkbox on every remote row**, in the same place and style as a local row's (`.swarm-agent-select-checkbox`).
- **A checkbox on each remote section header** that selects or clears that machine's agents (indeterminate when some are selected).
- **`Select all` stays this instance only.** Someone who clicks `Select all` and then `Stop` must never stop agents on other machines. Another machine's agents are added deliberately, by row or by header.
- **A stale section's rows can't be selected** (dimmed, checkbox disabled). They are about to disappear and an action on them will most likely fail.
- **The count says where the targets are:** `5 selected · 2 on other machines`. It never reports a bare number when some targets are remote.
- Selection survives the refresh. A selected agent that disappears is dropped from the selection, and the count shows it. The confirmation always re-resolves the list.

### 4.3 Saved groups stay local

A saved group stores block ids. Remote targets have no durable id, and a group that silently skips members that moved or went offline is worse than no group. For now the group picker applies to local members only, and `Save group` is disabled with a reason while the selection contains remote targets.

## 5. What an action does to each kind of target

### 5.1 The rule

An action is offered for a target only if its route can do it **and prove what it needs to**. A target the action cannot reach is not silently skipped and not silently attempted: it appears in the confirmation, marked, with the reason, and is left out.

| Action | This machine (other channel) | LAN | Cloud |
|---|---|---|---|
| **Stop** | Works today (loopback forward, same user). UI only. | Not offered (§6) | Not offered (§6) |
| **Broadcast** | Needs a small delivery route (§5.2) | Not offered until §6 | Not offered until §6 |
| **Save group** | Local only (§4.3) | | |

### 5.2 Broadcast to another channel on this machine

A new narrow route on the receiving instance, called over loopback with that instance's own `auth_key` from the shared registry (the trust model of the existing stop forward: same machine, same user).

- The sender supplies the target block id, the body, the `MSGID` and the `RECIPIENTS` count. The **receiving instance builds the `[BROADCAST:...]` header itself**, applies `sanitize_message` and delimiter neutralization to the body, and starts the turn with `BROADCAST_TURN_ORIGIN` (`Automated`). A caller cannot supply header text.
- `RECIPIENTS` counts every agent addressed on every machine and channel, so each agent knows how many others were told.
- Both instances audit the action: the sender with the existing `log_fleet_action_audit`, the receiver with its own entry naming the sending channel.
- A channel that predates the route answers 404 and is reported as "that channel is on an older build" for that target only.

### 5.3 Naming targets

The confirmation and the result panel stop printing block ids and print what the owner is deciding about:

```
Stop 4 agents?
  Korp            this machine
  Loap            narko · dev-fix-lan   [Windows] [this machine]
  Manoz           Area54                [macOS]   [LAN]      can't stop: no verified link yet
  Opaz            Area54                [macOS]   [LAN]      can't stop: no verified link yet
```

The unavailable targets are listed and excluded, and the button counts only the rest (`Stop 2`). Local agents are named too: this fixes the raw-id list that exists today. The result panel uses the same text per target, so a failure reads `Loap · narko · dev-fix-lan: ...` and not a UUID.

## 6. Acting on other machines: the part that needs a decision

Selecting agents on another machine is easy. Doing something to them needs the other machine to believe the request, and today it cannot.

**Why a broadcast cannot cross the boundary as it is.** A local broadcast is delivered as the user's own turn because the Swarm is the user's own window. The rule in `CLAUDE.md` and the broadcast spec §2.1 is that "verified" must mean cryptographically proven, and that "same account" or "same network" is never trusted. On the LAN, the credential that delivers a message is a key anything on the network can read, and it names an instance, not a person. If this Swarm sent a `[BROADCAST:FROM=user ...]` turn to a LAN peer on that credential, anything on the LAN could speak to that machine's agents with the owner's authority. So a remote broadcast must **not** be a user turn.

**What it can be.** A message from the operator of a verified instance, arriving under the existing jekt rules: `TRUST=lan-verified` or `TRUST=wan-verified`, normal tier, no claim of user authority. That needs the sender to sign as an *instance*:
- **Cloud:** the install keypair and the account's key directory already do this. It waits for the cloud tier of the Swarm (other-hosts Phase 3).
- **LAN:** only per-agent keys exist. It needs an instance-level signing key published the way the per-agent LAN keys are, which is a change to the LAN signing design and not part of this spec.

**Stop is harder.** A remote stop is destructive and cannot be undone from here. The agent-initiated bulk stop already gives the owner of each target a 15 s override window in their own window (`SPEC_AGENT_SELF_QUIT_2026_09_24.md` §6.5). A stop from the Swarm of another machine should use that same window on the target's screen, and so needs the same verified instance identity first.

Until then the rows are selectable (the owner's request), the confirmation says plainly what is not available and why, and nothing crosses a machine boundary on an unproven credential. §9 asks whether to build the verified path next.

## 7. Phases

| Phase | Scope | Needs | Verified on |
|---|---|---|---|
| **1: platform tag** | `os` in the TXT record and UDP reply; parse and sanitize; `RemoteChannel.os`; the tag on section headers. | AgentX's files, with their agreement | narko with a LAN peer on another OS |
| **2: selection and the honest confirmation** | `FleetTarget`, checkboxes on remote rows and headers, the count, §5.3 naming (local agents too), stale rows disabled, group save disabled with a remote target. Stop works for this machine's other channels. | nothing new on the wire | narko `stable` plus a `task dev` channel |
| **3: broadcast to other channels on this machine** | the §5.2 route and the forward in `fleet_broadcast_impl`. | a new loopback route | the same pair |
| **4: other machines** | instance-signed operator messages (cloud first, LAN after its key exists) and a stop with the target's override window. | §9 items 5 and 6 | narko with Area54 |

Phases 1 to 3 add no new trust. Phase 4 is where the security design is, and gets its own spec if the owner wants it.

## 8. Tests

- **Platform.** TXT and UDP round trip of `os`; a peer without `os` still lists with no tag; values outside `^[a-z0-9_-]{1,16}$` and unknown values give no tag; `windows`, `macos`, `linux` map to `Windows`, `macOS`, `Linux`; a Windows host with a WSL instance shows two sections with two tags.
- **Selection.** A remote row's checkbox toggles it; a section header selects exactly that machine's agents and shows the indeterminate state; `Select all` selects local agents only; a stale section's rows are disabled; a selected agent that disappears leaves the selection and the count; the count reads `N selected · M on other machines`; `Save group` is disabled with a remote target.
- **Confirmation and results.** Local and remote targets are named (no UUIDs); unavailable targets are listed with the reason and left out; the button counts only the available ones; the result panel uses the same names.
- **Stop.** A selected other-channel agent on this machine is stopped through the existing forward, and the failure of one target leaves the rest unaffected.
- **Broadcast (Phase 3).** The receiving instance builds the header; a caller-supplied header in the body is neutralized; `RECIPIENTS` counts every machine; an older channel gives a per-target failure; both sides audit.
- **Safety.** No remote target is ever sent a user-turn header over a LAN or cloud route. The renderer never receives a peer address or key.

## 9. Decisions for the owner

1. **Spelling.** `macOS` (Apple's spelling, proposed) or `MacOS` as you wrote it.
2. **Where the platform tag shows.** On each remote section's header and in confirmation and result lists (proposed), or on every row.
3. **`Select all`.** Stays this instance only (proposed), or includes every machine the Swarm lists.
4. **Rows you can select but not yet act on** (LAN, cloud): selectable, with the confirmation saying what is not available (proposed), or checkbox disabled until an action works.
5. **A message to another machine's agent.** Accept that it arrives as a verified operator message under the jekt rules, not as your user turn (proposed; §6), and whether to build the LAN instance key and cloud delivery as Phase 4.
6. **Stopping an agent on another machine.** Only with the target's own override window (proposed), or not at all.
7. **Groups.** Local members only for now (proposed), or store remote members as best-effort addresses.
