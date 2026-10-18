# RCA: the Swarm shows the AgentMux logo for remote agents that run on Claude

**Date:** 2026-10-18
**Status:** analysis — root cause found; §6.1–6.3 built in the PR that adds this report.
**Author:** Naki #2 (naki-naki-2), at the repo owner's request.
**Related:** `SPEC_SWARM_REFINEMENTS_2026_10_17.md` §3 (the provider icon on remote rows, #5057), `SPEC_SWARM_OTHER_HOSTS_AND_CHANNELS_2026_10_02.md` (remote sections).

## 1. Symptom

In the Swarm of a 0.59.32 instance, the agents of another AgentMux instance on the same machine all show the blue AgentMux block icon where their provider's icon belongs. The instance's own agents show the Claude logo correctly. Camper, Clamk and Loap are among the remote agents, and all of them are Claude agents.

## 2. What was checked (2026-10-18)

| Check | Result |
|---|---|
| Which instance hosts Camper, Clamk and Loap | Another instance on this machine, channel `local-main-…-dc494a12`, port 29708. Their entries in the host-global shared registry (`<home>/shared/agents/reactive/<agent>/<channel>.json`) say so. |
| What that instance runs | A portable **0.59.30** build (`0.59.30+g9a654394d`, built 2026-10-17 06:14). The viewing instance runs 0.59.32. |
| Whether 0.59.30 sends providers | No. `agent_providers` in `/agentmux/reactive/agent-names` came with #5057 (`7005dae3a`, merged 2026-10-17 08:09). `9a654394d` doesn't contain it; v0.59.30 doesn't, and v0.59.31 does. |
| Whether a current build sends them | Yes. The viewing instance's own `agent-names` lists `claude` for all seven of its agents. |
| Whether the provider is known on this machine anyway | Yes. The host-global agent definitions (`<home>/shared/agents/definitions/<uid>.json`, `data.provider`) say `claude` for Camper, Clamk and Loap. Every registry entry carries the agent's `uid` (srv `AgentEntry::uid`). All ten live entries matched a definition. |
| What the Swarm draws for a missing provider | `<ProviderLogo provider={row.agent.provider ?? "agentmux"} />` (`swarm-view.tsx`). `ProviderLogo` draws `agentmux` as the app icon. |

## 3. Root cause

Three things combine:

1. **The only source of a remote agent's provider is the peer's self-report.**
   - `swarm_remote::other_channels` takes `provider` from the peer's `agent-names` answer (`ChannelReport::providers`) and from nowhere else.
   - A peer older than #5057 reports none, and every release up to 0.59.30 is older. So every agent of such a peer has `provider: None`.
2. **"Unknown provider" is drawn as the AgentMux brand.**
   - `SPEC_SWARM_REFINEMENTS_2026_10_17.md` §3.2 chose `agent.provider ?? "agentmux"` on purpose: "same fallback, so rows stay aligned when a peer is from before this change or reports none".
   - The fallback keeps the layout, but it turns "we don't know" into a definite, wrong statement: that this is an AgentMux agent. The AgentMux logo is a real `ProviderLogo` value, not a placeholder.
3. **A better source was already on the machine and unused.**
   - For agents on this machine, the provider never needed the peer's word. The host-global definition store holds it, keyed by the `uid` that the registry entry already gives srv.
   - The design took the peer route only because LAN peers need it, and it applied the same rule to this machine's channels.

**Why it wasn't caught:**
- #5057 was tested with every instance on the new build: unit tests with a peer report that has `agent_providers`, and a render test with one agent with a provider and one without.
- The render test fixed the wrong behaviour as the expected one: "shows each agent's provider icon before its name, **and the AgentMux one when none was reported**" (`swarm-remote.render.test.tsx`).
- A mixed-version fleet, which is the normal state for at least a release cycle after any such change, wasn't part of the spec's test plan.

## 4. Scope

Affected:
- **This machine's other channels** running a build older than 0.59.31: every agent shows the AgentMux logo.
- **LAN peers older than 0.59.31:** the same. Unlike this machine's channels, nothing local knows their providers.
- **The local tree:** it uses the same fallback (`node.agentProvider ?? "agentmux"`, `swarm-view.tsx`). It only shows when a local agent's block has no `agentProvider` meta, which should be rare, but it is drawn as the same false claim.

Not affected:
- No decision is made on the provider. It is display only, as §3.2 of the spec says.
- No data is wrong; it is missing and drawn as if known.

## 5. Immediate remedy (no code)

Update the portable 0.59.30 instance to 0.59.32. Its `agent-names` answer then carries `agent_providers`, and its agents show the Claude logo within one Swarm poll.

## 6. Fix design

### 6.1 srv: this machine's channels take the provider from the definition store

- `register_swarm_other_instances` (`server/app_api/agent_io.rs`) already reads the shared registry in a `spawn_blocking`. In the same blocking task, it opens the shared definition store (`registry::resolve_shared_definitions_dir`, `DefinitionStore::open`, as `memory_dir_claims.rs` does). For each entry's non-empty `uid`, it reads `provider` and builds `uid → provider id`, keeping only ids that `providers::resolve_provider_alias` knows.
- `swarm_remote::other_channels` gains that map. An agent's provider is the peer's own report first, because it describes the live pane, then the definition's by `uid`, then none.
- LAN hosts are unchanged. Their definitions aren't on this machine, so their provider stays self-reported, from 0.59.31 on.
- Cost: one small JSON read per listed agent per Swarm poll, off the async workers. If that shows up, cache the map by the definition file's mtime.
- **Trust:** the definition store is this machine's own shared state, written by AgentMux instances of the same OS user. It is no less trustworthy than the registry entry the row already comes from, and it is still display only.

### 6.2 Frontend: "unknown" looks unknown

- `swarm-view.tsx`, remote and local rows:
  - With a provider, draw `<ProviderLogo provider={…}>` as today.
  - With none, draw a neutral placeholder of the same 16 px box. Use a muted outline circle (`fa-regular fa-circle`) in `--secondary-text-color`, so rows stay aligned.
  - The placeholder's title: "This agent's AgentMux doesn't say which provider it uses (an older version)". For a local row: "No provider recorded".
- The AgentMux logo stays for an agent whose provider really is `agentmux`, if such an id is ever set, and is never used as a fallback.

### 6.3 Spec and tests

- **Spec:** amend `SPEC_SWARM_REFINEMENTS_2026_10_17.md` §3.2:
  - drop "same fallback" in favour of §6.2's placeholder;
  - note §6.1's source for this machine's channels;
  - add a mixed-version case to §3.3.
- **srv unit tests** (`swarm_remote.rs`):
  - a channel whose report has no providers gets them from the definitions by `uid`;
  - a provider in the report wins over the definition's;
  - an unknown definition provider is dropped;
  - an entry with an empty `uid` gets none.
- **srv route test:** `swarm.other-instances`, with a fake older peer (no `agent_providers`) and a definition on disk, returns `provider: "claude"`.
- **Frontend:**
  - replace the render test's "the AgentMux one when none was reported" with "a neutral placeholder, with its title, and no AgentMux logo";
  - add the same for a local row without `agentProvider`.

### 6.4 Not in this fix

- **The peer's version in the section header** (e.g. "0.59.30 · older"): it would explain gaps like this one at a glance, but needs the version advertised. That would be a separate `agent-names` field, which needs the owner's decision per the route's doc comment.
- **Providers for LAN peers older than 0.59.31:** no local source exists. They show the placeholder until they upgrade.
