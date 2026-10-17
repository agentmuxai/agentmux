# Spec: Swarm refinements — the selection count, provider icons on remote agents, broadcast to remote agents

**Status:** active. §2 (the selection count) and §5 (one list) shipped in #5055, §3 (provider icons) in #5057; §4.2 (LAN broadcast) follows in its own PR. The owner said "ok lets build it" (2026-10-17) to the recommendations in §7: D1, D2 and D5 yes, D3 not now.
**Date:** 2026-10-17
**Author:** Lazo
**Verified against:** `agentmux` `main` @ `ae0fbb2b3` (2026-10-17), by reading code.
**Builds on:** [SPEC_SWARM_OTHER_HOSTS_AND_CHANNELS_2026_10_02.md](SPEC_SWARM_OTHER_HOSTS_AND_CHANNELS_2026_10_02.md), [SPEC_SWARM_REMOTE_AGENTS_PLATFORM_TAG_AND_SELECTION_2026_10_03.md](SPEC_SWARM_REMOTE_AGENTS_PLATFORM_TAG_AND_SELECTION_2026_10_03.md) (§6 especially), [SPEC_SWARM_BROADCAST_AS_USER_MESSAGE_2026_10_01.md](SPEC_SWARM_BROADCAST_AS_USER_MESSAGE_2026_10_01.md).

## 1. Ask

The owner, 2026-10-17: "we want [to] refine the swarm. … I have 2 agents selected, but broadcast says 'send to 4', there is a state mismatch there. Let's get the provider icon to the left of each remote agent. Does the broadcast send to remote agents currently? Get that working if not." Then: "write a spec to file that we will continue refining."

And, later the same day: "remove the separator between the 2 types of agents in the swarm, just make it one long list with the local ones first followed by the remote ones."

Four items:
1. The broadcast count disagrees with the agents the owner sees selected (§2).
2. Remote agents have no provider icon (§3).
3. Broadcast to remote agents (§4).
4. One list: local agents, then remote ones, no separator (§5).

## 2. The selection count

### 2.1 Today

The selection is one in-memory set of keys, `SwarmModel._selectedBlockIds` (`frontend/app/view/swarm/swarm-model.ts`). A key is a block id for this instance's agents and for this machine's other channels, and `remote:<section>/<name>` for an agent on another machine (`remoteAgentKey`, `swarm-fleet-targets.ts`).

The toolbar shows two numbers, both from that set (`swarm-fleet-toolbar.tsx`):
- "**N selected**": `selected().size`.
- "Message to send to **M** agents…": every selected key except those whose target can't receive a broadcast. A key that matches **no known target** counts as reachable (`if (!t || !unavailableReason(t, action)) n++`).

M can't exceed N, so "2 checked, send to 4" means the set holds **keys with no checked checkbox on screen**. Two ways that happens today:

1. **A local agent that leaves the tree stays selected.** `loadOtherInstances` drops a selected *remote* agent that disappears or goes stale. Nothing drops a selected *local* agent whose pane closes, or whose row otherwise leaves the tree. Its key stays in the set with no row to show it. The toolbar counts it in both numbers. A broadcast sends it to the srv, which fails it per target ("no longer listed" is never said up front). `partitionTargets` passes unknown keys through as reachable, by design ("the action reports it per target").
2. **A collapsed machine section hides its selected agents.** A machine header's checkbox selects all of that machine's agents (`setManySelected`). Collapse the section and those agents are still selected but not visible. The collapsed header shows only the agent count, not how many are selected.

Not the cause: the selection isn't persisted, so stale keys don't survive a reload.

Either case gives the owner's symptom. Which one they hit isn't known: both get fixed.

### 2.2 Change

One rule: **the toolbar's numbers count only targets the Swarm currently lists, and every selected target is visible or said to be hidden.**

- **Prune on every tree change, local as well as remote.** The selection keeps only keys in `fleetTargets()` (this instance's tree plus the non-stale remote agents). It's done in one place, a model effect over `buildTree()` and `otherInstancesAtom()`, replacing the remote-only pruning in `loadOtherInstances`.
- **`partitionTargets` treats an unknown key as unreachable**, with the reason "no longer listed". With pruning this should never happen. If it does, the result panel names it rather than sending it to the srv.
- **A collapsed section says what it hides:** "3 agents · 2 selected" in its header. Only if machine sections stay: §5 removes them (D5).
- **The toolbar names any gap between N and M**, e.g. "4 selected · send to 2 (2 on other machines can't be messaged)". The reason text is the one the result panel already uses (`unavailableReason`).

### 2.3 Tests

- Selecting a local agent, then removing it from the tree (the RPC mock drops its block), takes it out of the selection, and both toolbar numbers drop.
- A collapsed section with selected agents shows the selected count in its header.
- An unknown key in `partitionTargets` lands in `unreachable` with "no longer listed".
- The toolbar's text names the unreachable count and reason when M < N.

## 3. A provider icon on each remote agent

### 3.1 Today

A local agent row shows `<ProviderLogo provider={node.agentProvider ?? "agentmux"} size={16} />` (`swarm-view.tsx`). A remote agent row shows a checkbox, its name and a status chip. The data has no provider:
- `RemoteAgent` (srv `backend/swarm_remote.rs`, frontend `swarm-remote.ts`) has `name`, `block_id`, `status` and `line`.
- Its state and line come from each instance's `/agentmux/reactive/agent-names` route. A LAN peer is polled by the discovery loop, and this machine's other channels are asked over loopback (`channel_statuses`). The route returns `agents`, `agent_kinds`, `agent_status` and `agent_lines`.
- That route is readable with the LAN broadcast key, so **its output is public to anyone on the local network**. Its doc comment says not to extend it without the owner's decision, and records one decision per field (`server/reactive.rs`, `handle_reactive_agent_names`).

### 3.2 Change

- **srv (each instance, as a sender):** `agent-names` adds `agent_providers: {name: provider}`, the provider id the local tree already has (`claude`, `codex`, `gemini`, `opencode`, …). An agent with none is left out, like the other maps. **Needs decision D1.**
- **srv (as a reader):** `parse_agent_names_body` reads it, capped like the other maps (known names only, bounded length). `RemoteAgent` gains `provider: Option<String>`, serialized only when present.
- **Frontend:** the remote row shows `<ProviderLogo provider={agent.provider ?? "agentmux"} size={16} />` before the name, as the local row does (same fallback, so rows stay aligned when a peer is from before this change or reports none). A provider id `ProviderLogo` doesn't know draws its first letter in a circle, as on local rows.
- Display only: the provider is self-reported by the peer, like `os`, and nothing decides anything on it.

### 3.3 Tests

- srv: `agent-names` includes `agent_providers` for agents with a provider. The parser keeps only listed names and drops oversized values. A body without the field parses as before.
- Frontend: a remote row with a provider renders its logo before the name, and one without renders the placeholder.

## 4. Broadcast to remote agents

### 4.1 Today

| Where the agent is | Broadcast reaches it? | How |
|---|---|---|
| This AgentMux | yes | delivered as the user's own turn, with the `[BROADCAST:FROM=user VIA=swarm …]` header (broadcast spec) |
| Another channel on this machine | **yes** | forwarded over loopback, with the registry's loopback auth (`forward_broadcast_to_channel`, `server/app_api/fleet.rs`); the target AgentMux writes the header itself |
| Another machine on the LAN | **no** | listed and selectable; the result says "can't message: no verified link to other machines yet" (`unavailableReason`) |
| A cloud-only install | **no** (not listed) | the Swarm's cloud tier (other-hosts Phase 3) isn't built |

The LAN refusal is deliberate (SPEC_SWARM_REMOTE_AGENTS_PLATFORM_TAG_AND_SELECTION §6). On the LAN, the credential that delivers a message is a key anything on the network can read, and it names an instance, not a person. A `[BROADCAST:FROM=user …]` turn sent on it would let anything on the LAN speak to that machine's agents with the owner's authority. So a remote broadcast **must not** arrive as a user turn.

### 4.2 Change: a remote broadcast arrives as a signed message, not a user turn

On another machine, a broadcast arrives the way an agent's message does: as a jekt, checked against a signature, at the normal tier, from the operator of the sending instance, with no claim of user authority. The receiving agent reads it as information from a verified sender, not as its owner speaking.

- **Sender identity:** the sending **instance** signs, not an agent. The cloud tier has this already (the install keypair and the account's key directory). The LAN has only per-agent keys (SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15), so this needs an **instance LAN key**, published the way per-agent LAN keys are (the `/agentmux/reactive/agent?id=` pubkey half), which the receiver checks. **Needs decision D2.**
- **What the agent sees:** a jekt with `FROM=<owner's instance>@<host> VIA=swarm`, `TRUST=lan-verified` (or `wan-verified`), normal tier. The text says it's a Swarm broadcast from that instance, so the agent knows it's fleet-wide, not addressed to it alone.
- **The result panel** reports each remote target's delivery like a local one: delivered, queued, held, or failed with the receiver's reason.
- **Stop stays out of scope.** A remote stop is destructive. It needs the 15 s override window on the target's own screen (SPEC_AGENT_SELF_QUIT §6.5) as well as the verified identity. The toolbar keeps saying why it's unavailable.

### 4.3 Option, for the owner to weigh (D3)

If the owner's machines are **paired** (the Pair device flow), a pairing could be proven strongly enough to let a broadcast from a paired machine arrive as the owner's own turn, as on this machine. That turns pairing into "this machine speaks for me". It's a larger trust change than §4.2 and needs its own threat model, so it isn't recommended for the first step.

### 4.4 Tests

- A broadcast to a LAN agent goes out signed with the instance key. The receiver verifies it and delivers a `TRUST=lan-verified` jekt, never a `[BROADCAST:FROM=user` turn.
- A forged or unsigned request is refused, and the sender's result panel shows the receiver's reason.
- A peer from before this change (no instance key published) is still reported "can't message", with that reason.

## 5. One list: local agents first, then remote ones

### 5.1 Today

The Swarm renders this instance's tree, then `OtherInstanceSections` (`swarm-view.tsx`) below it. They're split by `.swarm-remote`'s top border and 10 px margin (`swarm-view.scss`). Each other instance then gets its own section: a header row with a select-all checkbox, a collapse chevron, the title ("narko · dev-fix-lan"), the platform tag and the badge ("this machine", "LAN", "cloud"). The agents are indented under it.

### 5.2 Change

- **No separator:** `.swarm-remote` loses its top border, margin and padding. The first remote agent follows the last local one with the same spacing local agent groups have between them.
- **No machine headers:** each remote agent is a row in the same list, after every local agent. Order: this machine's other channels, then LAN hosts, then cloud (as now), then by machine title, then by agent name.
- **Where it runs moves onto the row:** after the name (and status chip), a muted tag says where the agent is, e.g. "narko · LAN · macOS". A stale agent's row is dimmed, its checkbox disabled, and its tag adds "seen 4m ago", as its section header does today.
- **What goes with the headers:** the per-machine select-all and collapse. Select all in the toolbar selects every listed agent, local and remote, which it doesn't today (it selects local agents only). §2's collapsed-section count also goes, as there are no collapsed sections. **Needs decision D5.**

### 5.3 Tests

- The list renders local agent groups, then remote rows, with no `.swarm-remote` border element and no machine header.
- A remote row shows its "host · badge · platform" tag. A stale row is dimmed and can't be selected.
- Select all selects local and non-stale remote agents.

## 6. Phases

1. **Selection count** (§2) **and one list** (§5): frontend only, one PR. Needs D5 only for the headers.
2. **Provider icons** (§3): srv plus frontend, one PR, after D1.
3. **LAN broadcast** (§4.2): the instance LAN key, its publication, signed send and verified receive, then the Swarm wiring. After D2. Two PRs: the key and signing, then the Swarm.
4. **Cloud broadcast:** waits on the Swarm's cloud tier (other-hosts Phase 3).

## 7. Decisions for the owner

Decided 2026-10-17 ("ok lets build it"): the recommendation on each.

- **D1. Add `agent_providers` to the LAN-readable `agent-names` route?** Anyone on the local network could then see which provider each agent runs. *Recommended: yes.* It's the same kind of fact as `agent_kinds`, it reveals no content, and agent names are already listed.
- **D2. A remote broadcast arrives as a verified jekt from the sending instance, not as the user's own turn.** That means building an instance-level LAN signing key. *Recommended: yes*, as §4.2.
- **D3. Paired machines' broadcasts as the owner's own turn (§4.3)?** *Recommended: not now.* Revisit after D2 ships, with a threat model.
- **D4. Anything else to refine in the Swarm** while we're in it? The owner said "for one bug", so more may come. Add them as new sections before §6.
- **D5. One list without machine headers (§5.2)?** That drops the per-machine select-all and collapse, and Select all then covers remote agents too. *Recommended: yes.* The alternative is to keep a thin, unbordered machine label line above each machine's agents (no checkbox, no collapse), which keeps the grouping without the separator.
