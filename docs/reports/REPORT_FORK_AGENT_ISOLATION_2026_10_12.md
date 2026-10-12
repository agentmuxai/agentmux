# Report: forked ("iterative") agents — a fork showing up as a pane tab, and how isolated forks really are

**Status:** active — F2 and F3 are fixed in the change that adds this report (§3 items 2 and 3, as built in §5); F1, F4 and F5 follow in their own changes.
**Date:** 2026-10-12 · **Author:** agent2
**Trigger:** the repo owner: *"I load an agent "Loap #3" in Tab 4, and somehow Loap from Tab 1 became a pane tab inside of Loap #3's pane. Meanwhile, if I click between Loap and Loap #3 in the pane tabs, it switches window tabs to get to the respective panes. lets make sure our iterative agents have solid support and are isolated."*
**Written against:** `main` @ `9088493f5` (v0.59.21). The live evidence is from a local v0.59.20 build's srv and host logs, 2026-10-12 17:53–17:57 UTC.

"Iterative agents" here are **forks**: `ForkAgentDefinitionCommand` makes a new agent definition linked to its source by `parent_id`, auto-named "*Root* #N". "Loap #3" is a fork of "Loap #2", itself a fork of "Loap".

## 1. What happened

Nothing moved Loap into Loap #3's pane. Tab 4's layout (`bb3fe4d0`) was saved five times between 10:16 and 17:57, and Loap #3's block (`8cbb554f`) was alone in its own tile every time; the layout never held a stack.

What the user saw is a **fork pill**. Every agent pane's tab strip adds, after its own pane tabs, one pill per *other open member of its fork lineage*, wherever that member is open in the window (`frontend/app/view/agent/agent-pane-chrome-model.ts:86-111`, `extraTabs`). Loap is the root of Loap #3's lineage and was open in Tab 1, so Loap #3's pane showed a "Loap" pill. It is drawn exactly like a pane tab.

- **Clicking it** reveals Loap's block (`revealBlockLocally`, `chrome-model.ts:136-139`), which switches to the window tab holding it (`frontend/app/util/reveal-block.ts:100-126`). That is the "switches window tabs" behaviour; Loap's own pane shows a matching "Loap #3" pill back.
- **Its ×** calls `closeAgentTab` (`chrome-model.ts:156-158`), which looks the block up in *this* pane's window-tab layout (`close-agent-tab.ts:54-56`). A block in another window tab isn't there, so the × silently does nothing. For a fork in another pane of the same window tab it closes that pane's fork.

The specs disagree with each other here. `SPEC_AGENT_PANE_FORKS_AND_AUX_PINS_2026_06_15.md` §6 has forks as a *fork set hosted in one pane*, switching "view-level". `SPEC_AGENT_PANE_HOVER_CLOSE_FOCUS_REFINEMENTS_2026_09_23.md` describes these pills as "forks … open as their own top-level pane … Clicking one jumps focus to that pane. Its × closes the fork there", without considering window tabs. `SPEC_REVEAL_BLOCK_ONE_PATH_2026_09_27.md` then made the click cross window tabs on purpose ("the target may be in another window tab"); the × was never updated to match.

## 2. Findings

Ranked by impact. "Live" means seen in the 2026-10-12 logs; the rest are from reading the code.

### F1. A fork pill looks like a pane tab, and crosses window tabs (the reported bug)

- A pill for an agent in another window tab is indistinguishable from a tab of this pane. Users read it as their agent having been moved into this pane, as here.
- Clicking it changes window tab with no warning.
- Its × does nothing when the fork is in another window tab (above), contrary to the 09-23 spec's "Its × closes the fork there".
- `getOpenDefinitionMap` keeps one block per definition, the last registered (`agent-pane-state-store.ts:499-506`). A definition open in two panes yields one pill, for whichever registered last.

### F2. Every agent pane wipes its own launch environment when it mounts — all agents, not only forks (live)

`agent-view.tsx:680-686` runs the launch flow on every mount. `flows/launch-flow.ts:276-282` then writes `"cmd:env": effectiveAuthEnv`, only the auth variables. Block meta merges replace a key whole (`crates/srv/src/backend/obj.rs` `merge_meta`), so this drops what the launch put there: `AGENTMUX_AGENT_ID`, `AGENTMUX_AGENT_SLUG`, `AGENTMUX_INSTANCE_SLUG`, `AGENTMUX_AGENT_DISPLAY`, `GH_CONFIG_DIR`, the `GIT_*` identity and the agent's own `env` lines. `launch-flow.test.ts:339` asserts exactly the replaced value.

srv restores some of these at spawn (`agent_handlers/input.rs`: the slug from the stored definition, `GH_CONFIG_DIR` via `gh_guard`), but `AGENTMUX_AGENT_ID` falls back to the display name (`input.rs:619-629`). So an agent's MuxBus identity changes from its slug to its display name at its next spawn after any pane mount: fresh launch, picker switch, `agent.open`, restore at startup.

Live evidence:
- Loap #3: srv registered `loap-loap-2-loap-3` at 17:54:19 (epoch 6, before the frontend's write landed), then `Loap #3` at 17:54:33 and 17:55:27 (epochs 7 and 8).
- The same flip is in the 2026-10-11 log for Agent2: `agent2` at 19:08:46, then `Agent2` at 19:08:49. Agent2's name happens to be a valid id, so nothing failed visibly.

### F3. A fork's own name is not a valid agent id, so forks drop out of Swarm and MuxBus (live)

Agent ids allow only `A-Z a-z 0-9 _ -`, up to 64 characters (`crates/srv/src/backend/reactive/sanitize.rs:133-140`). A fork's automatic name "*Root* #N" has a space and a `#`. With F2 making the display name the id, every registration by name fails:
- the pane's own registration on mount, which is how Swarm sees it (`agent-view.tsx:264` → `termagent.ts`); live: `[reactive] failed to register agent: invalid agent ID: Loap #3`;
- the spawn-time MuxBus registration (`persistent/spawn.rs`); live: `muxbus: persistent auto-register failed … invalid agent ID: Loap #3` on both respawns;
- the per-turn registration (`input.rs:1300-1357`), skipped silently.

Effects: the fork is missing from Swarm; jekts and `SendMessage` to it find no registration; memory lookups keyed by `AGENTMUX_AGENT_ID` miss; its git email becomes `loap #3@agentmux.local`. Its MCP server still sends as the slug, so its *outgoing* messages work, which hides the problem.

### F4. Fork slugs chain and grow

The fork's slug is `format!("{}-{}", source.slug, derive_slug(branch_label))` (`agent_handlers/template.rs:414-421`). A fork of a fork carries every generation's label: `loap` → `loap-loap-2` → `loap-loap-2-loap-3`. Nothing trims it, so a long name passes 64 characters within a few generations (e.g. "Research Assistant" at the third fork), and then even slug-based registration fails. The automatic number counts the lineage's existing forks (`template.rs:70-84`), so deleting a fork can hand its number out again, and typed labels are never checked for uniqueness; duplicate names collide in name-keyed registries.

### F5. Starting a fork from the parent's conversation is fragile

From the code; not reproduced live.
- The fork runs in its own working directory, and Claude finds sessions per project directory. Nothing copies the parent's session file there, so `--resume <parent session>` may not find it.
- `--fork-session` is one-shot: every send rebuilds `cmd:args` without it (`useAgentCommands.ts:905-910`). Unless the launch-time spawn happens at once, the first send runs a plain `--resume` of the parent's session, which continues the parent's own session if it is reachable.
- With the parent still running on that session (the usual case for a quick fork), the duplicate-session guard refuses the fork's spawn ("already open in another pane"), with no fork exception (`persistent/spawn.rs:154-157`).

### F6. Lineage edges

- Deleting a parent leaves its forks pointing at a missing `parent_id`, so the lineage splits into separate pill sets, and later suggested names become "Loap #3 #N" (`template.rs:45-76`).
- A fork copies the parent's `ui:color` (`template.rs:468-481`), so parent and forks share a border colour. Together with F1's identical-looking pills, they are hard to tell apart.

### What is properly isolated

Each fork has its own working directory (`loap-3-0822g`), `GH_CONFIG_DIR` (from its slug), agent MCP config, Claude project directory and native memory, transcript zone, continuity chain, lease and admission epoch, and its own empty memory bundle. Skills and content files are copied, not shared. The account's credential directory is shared on purpose.

## 3. Recommendations

1. **Fork pills (F1).** Show pills only for forks open in this pane's own window tab, or mark ones elsewhere clearly as "in Tab 1" with no ×. Make the × work wherever a pill is shown: close through the block's own window tab's layout. Decide between the two specs and record it.
2. **Stop wiping `cmd:env` (F2).** The launch flow should merge its auth variables into the block's existing `cmd:env`, not replace the key, or keep them under a key of their own that srv merges at spawn. One small change; it fixes the identity flip for every agent. Add a test that a pane mount keeps `AGENTMUX_AGENT_ID`.
3. **Register by the stable id, never the display name (F2, F3).** The frontend should register the slug; srv's fallback in `input.rs` should derive the id from the stored slug, never the raw display name. A display name with spaces or `#` should never reach `validate_agent_id`.
4. **Fork slugs (F4).** Derive a fork's slug from the lineage root's slug plus its own number (`loap-3`), not by chaining; cap the length; number forks by the highest existing number plus one, not a count; refuse a duplicate label.
5. **Fork conversations (F5).** At fork time, copy the parent's session file into the fork's project directory and fork from that copy, the way `continuity_relocate` already moves sessions across logins. Keep the fork intent until the first spawn instead of losing it on the first send, and let a fork's spawn through the duplicate-session guard when it forks.
6. **Tests for iterative agents.** One end-to-end test that forks twice and checks each generation for: a valid, distinct, stable agent id; Swarm and MuxBus registration; a distinct working directory and `GH_CONFIG_DIR`; and a conversation that starts from the parent's and doesn't write to it.

Recommendations 2 and 3 are the smallest changes with the largest effect, and they are needed before forks can receive messages at all.

## 4. Evidence

| Time (UTC) | Source | Event |
|---|---|---|
| 10:16:50 → 17:57:08 | host log, `object.UpdateObject layout bb3fe4d0` (v1–v5) | Tab 4's layout: `8cbb554f` (Loap #3) in its own tile; no stack in any version |
| 17:54:19.227 | host | `Launching agent definition Loap #3 (claude)` |
| 17:54:19.383 | srv | `agent_admission.granted agent=loap-loap-2-loap-3` (epoch 6) |
| 17:54:19.453 | host | `[reactive] failed to register agent: invalid agent ID: Loap #3` |
| 17:54:20.472 | srv | `muxbus: auto-registered persistent agent agent_id=loap-loap-2-loap-3` |
| 17:54:33.797 | srv | `agent_admission.granted agent=Loap #3` (epoch 7) |
| 17:54:35.373, 17:55:28.627 | srv | `muxbus: persistent auto-register failed … invalid agent ID: Loap #3` |

## 5. As built: F2 and F3

- **No path replaces `cmd:env` any more.** `frontend/app/store/block-meta.ts` `mergeBlockEnv` / `blockEnv` set some variables and keep the rest. The launch flow, the provider login's pane refresh and the Armory bind all go through them; the bind already merged by hand, the other two replaced the key.
- **`AGENTMUX_AGENT_ID` means the row's slug wherever a row exists.** That is what the frontend launch and `agent.open` already set, and what the name-keyed signing keys are filed under: the agent's MCP uses its fetched keys only when the row's slug is exactly its `AGENTMUX_AGENT_ID` (`crates/mcp/src/self_keys.rs`, identity M4d). srv's fallback for a `cmd:env` without one (`carry_agent_id_env`, `agent_handlers/input.rs`) now takes the row's slug, and the display name only for a pane with no row (quick launch).
- **One rule turns a name into a valid agent id:** `agentmux_common::slug::agent_route_name`, a valid id unchanged, otherwise `definition_slug`'s rule without lowercasing (`Loap #3` → `Loap-3`). The registry's display binding (`register_agent_full`), the HTTP register and unregister, a message's target and srv's rowless fallback all use it; `validate_agent_id` delegates to the same module's `is_agent_id`.

**Visible change.** An agent's `AGENTMUX_AGENT_ID` had been its display name (`Agent2`) only because F2 wiped the launch's slug at the first pane mount. With F2 fixed it stays the slug (`agent2`), so gh-agent signs PR titles and commits in the slug's casing (`agent2@narko`). The PR-title check accepts either casing. `SPEC_PR_TITLE_AGENT_HOST_PREFIX_2026_08_22.md`'s natural casing for titles conflicts with M4d's exact-slug signing; signing wins here, and a display-cased signature would come from `AGENTMUX_AGENT_DISPLAY` in gh-agent, not from changing the id.
