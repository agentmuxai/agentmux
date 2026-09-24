# SPEC: Memory follows the agent — an agent's memory survives upgrades, channel changes and account switches, as its conversation now does

**Date:** 2026-09-24
**Status:** draft. This is a skeleton; research is in progress, and the
design will follow. An adversarial review comes before anything is built.
**Trigger:** the repo owner, on 2026-09-24, after upgrading to 0.57.2 and
reopening this agent in the new build:
> "we are doing work right now regarding conversation history, more
> robust, so it follows. we also want something similar for memory. can
> you investigate that, write spec to file, you should already see PRs in
> progress from agent3 / agentx"

and, in a follow-up:
> "there is also work (perhaps 3 weeks ago) regarding cloud-shared global
> memory .. we likely want to extend that to personal agent memory too
> ...we also want to refine the armory .. the global memory should match
> the file tile look of an agent's personal memory. tiles first, then
> expand to full. also pin the text editor to the bottom of the pane. move
> anything below to the top."

**Related (in progress):**
- The continuity series: #3673, #3674, #3676, #3677, #3678.
- `SPEC_DURABLE_CONVERSATION_MEMORY_2026_09_23.md`.
- #3693, SearchHistory correctness (agent3).
- `SPEC_CROSS_CHANNEL_AGENT_HISTORY_RESOLUTION_2026_09_21.md`.
- `SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md`.
- `SPEC_CROSS_INSTANCE_GLOBAL_MEMORY_SYNC_2026_09_20.md`.

## 0. Scope

1. **Memory follows the agent.** An agent keeps every kind of memory it
   has across version upgrades, channel changes, provider account
   switches and renames, the way its conversation history now does.
2. **Cloud-shared Personal Memory.** Extend the existing cloud sync for
   Global Memory to each agent's Personal Memory, so an agent's memory is
   the same on every AgentMux instance where that agent runs.
3. **The Armory's Global Memory view** matches the file-tile look of an
   agent's Personal Memory:
   - tiles first;
   - a tile expands to the full view of that memory.
4. **The memory editor pane:**
   - the text editor is pinned to the bottom of the pane;
   - anything that currently sits below it moves to the top.

## 0.1 The question

When an agent is opened in a new build, a new channel, or under a
different provider account, does it keep its memory? That covers every
kind of memory an agent has:
- the provider's native memory, e.g. Claude's auto-memory directory;
- AgentMux Personal Memory;
- Global Memory;
- the instruction files it is launched with.

And how should the work that makes conversation history "follow" the
agent be mirrored for memory?

## 1. Research (in progress)

1.1 How conversation history now follows the agent (the continuity series)
1.2 Every memory store an agent has, where it lives, and what keys it
1.3 What happens to each on a version upgrade, a channel change, an
account switch, and a rename — measured
1.4 Prior specs, and where they stand
1.5 Cloud-shared Global Memory: what was built, and how Personal Memory
could reuse it
1.6 The Armory today: Personal Memory tiles, the Global Memory view, and
the editor layout

## 2. Design (to follow)

## 3. Rollout and tests (to follow)
