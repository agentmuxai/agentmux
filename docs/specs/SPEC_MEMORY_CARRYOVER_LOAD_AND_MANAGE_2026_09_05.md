# Memory carry-over: loading and management across the three agent-awareness cases

**Status:** proposed
**Date:** 2026-09-05 (empirical findings added 2026-09-06)
**Author:** Manoz@Area54

**Motivating ask (repo owner, live session):**

> we want to refine how memory is loaded and managed. There are a couple
> scenarios: 1) creating a new agent 2) loading an agent already existed
> 3) after compaction. those are the 3 main cases where we need to ensure
> memory carries over for an agent to be aware of

> **Revision note.** The first draft of this spec listed four unknowns and
> designed around them. All four were then resolved empirically against
> live data on this machine (§2). **Two of the findings invert the draft's
> conclusions** — compaction is *not* where memory is lost, and the memory
> AgentMux composes is empty in practice. §3 and §4 are rewritten
> accordingly. The measurements are recorded in full because several
> contradict a previously-accepted conclusion in
> `docs/analysis/TOKEN_TAX_FOLLOWUP_2026_07_04.md` (§2.7).

---

## 1. The two memory systems

**System A — AgentMux composed memory.** On every `agent.open`,
`write_agent_config_files` (`agentmux-srv/src/server/app_api/agent_open.rs:712`)
loads `agent_content_get_all` into a `content_map` (`soul`, `agentmd`,
`memory`, …), injects global brain bundles on top of `content_map["memory"]`
(`agent_open.rs:757-769`), and `agent_config::build_config_files`
(`agent_config.rs:82-135`) composes `Soul → AgentMD → # Memory → # Available
Skills` into the provider's native startup file. For Claude that write goes
through `write_claude_md_respecting_ownership` (`agent_config.rs:1106`),
which never overwrites a foreign `CLAUDE.md` — it writes
`.claude/AGENTMUX_MEMORY.md` and offers a one-time `@import` line instead.

**System B — CLI auto-memory.** Claude Code's own memory directory
(`MEMORY.md` index + one file per memory), written by the agent via
`MemoryWrite`. AgentMux tracks it (`db_agent_native_memory`, drift detection,
versioning, retention, a `NativeMemoryList` RPC) but does **not** compose it
into anything — it reaches the model natively.

## 2. Empirical findings (measured 2026-09-06, this machine)

### 2.1 Compaction rewrites the message list only — memory is not in it

Measured against real Claude Code session transcripts under
`~/.agentmux/shared/identities/*/claude/projects/*/*.jsonl`.

The boundary record is `{"type":"system","subtype":"compact_boundary",…,
"compactMetadata":{"trigger":"auto","preTokens":934469,…,"preservedSegment":…}}`.
The very next record is a `type:"user"` message whose content begins *"This
session is being continued from a previous conversation that ran out of
context. The summary below covers…"*. So compaction **replaces the message
list with a summary message**.

`CLAUDE.md` occurs **0 times in that transcript — before *or* after the
boundary**. It is not in the message list at all; it is delivered in the
system prompt, which is rebuilt per request. Compaction therefore *cannot*
remove it. This is structural, not incidental.

### 2.2 Both memory paths survive compaction

Follows from §2.1 for System A. For System B, independently corroborated by
`docs/analysis/TOKEN_TAX_ANALYSIS_2026_06_19.md:111`, whose table records CLI
auto-memory loading at **"Fresh session + post-compact"**.

Directly confirmed first-hand: the session that produced this spec is itself
a post-compaction continuation (it opened with the §2.1 summary message), and
both the composed file and the `MEMORY.md` index were present in context
throughout.

**This is the finding that inverts the draft.** Case 3 is not where memory is
lost.

### 2.3 The `PreCompact` hook cannot inject anything

`agentmux-bashwrap/src/precompact.rs` is already registered in every agent's
`.claude/settings.json` (verified live). Its own contract: *"unlike
`PreToolUse`'s `passthrough()`, `PreCompact` must exit 0 with **no stdout
output at all** — not even `{}`"*. It is observe-only. Any injection has to
happen elsewhere.

### 2.4 Compaction signalling is Claude-only

`compact_boundary` appears in exactly one translator —
`agentmux-srv/src/agents/translator/claude.rs`. The other six providers with
a `startup_instructions_filename` (codex/AGENTS.md, gemini/GEMINI.md,
qwen/QWEN.md, pi/.pi/APPEND_SYSTEM.md, …; kimi is `None`) emit no compaction
signal at all. Anything keyed on the boundary is Claude-only by construction.

### 2.5 System A carries no memory in practice — for any agent

This is the significant finding.

`db_agent_content` (channel `objects.db`) contains exactly four content types
across all 8 agents:

| content_type | rows | non-empty |
|---|---|---|
| `env` | 8 | 8 |
| `startup` | 8 | 8 |
| `ui:color` | 2 | 2 |
| `ui:zoom` | 2 | 2 |

There are **no `soul`, `agentmd`, or `memory` rows at all** — not empty ones,
absent ones. And `select count(*) from db_bundles where is_global=1` returns
**0**, so the global-brain injection has nothing to add either.

The composed output confirms it end-to-end: `.claude/AGENTMUX_MEMORY.md` is
**byte-identical at 1089 bytes across three separate agents** (`manoz-0803a`,
`agenta-07017`, `posa-08030`), contains only the Skills index, and has **zero
`# Memory` sections**.

So System A's memory pipeline is *functioning* — it simply has nothing to
carry. Every agent on this machine is running with an empty AgentMux memory.

Note this channel is a fresh per-build channel, and `db_bundles` is **not**
globalized (CLAUDE.md's own data-isolation notes), so global-brain content
does not follow a new build. That explains the 0 — and means it is a
*structural* consequence of the isolation model, not a one-off.

### 2.6 What is and isn't global — measured, because it is easy to get backwards

Storage scope decides which memory survives a new build, so it was measured
directly rather than reasoned from the isolation notes:

| Data | Scope | Survives a fresh build channel? |
|---|---|---|
| Conversations / transcripts | global — `~/.agentmux/shared/identities/…` (12,018 `.jsonl` present) | **Yes** |
| Native memory, System B | **global** — see below | **Yes** |
| Global brain / bundles, System A | per-channel `db_bundles` | **No** |
| Agent definitions / registry | global | Yes |

The System B row is the one worth stating precisely, because the directory
layout suggests the opposite. An agent's memory appears at *both*
`channels/<channel>/identities/<id>/…/memory/` and
`shared/identities/<id>/…/memory/`, which reads like a per-channel copy that
could drift. It is not: `stat` reports **the same inode
(`4503599628262865`) for `MEMORY.md` at both paths** — one physical file
reached through a link. (`find -type d` misses the channel path while `ls`
resolves it, which is the giveaway.) Content and mtimes are identical
because they are the same bytes, not because something syncs them.

**So the only memory that does not carry over is System A's — exactly the
one measured empty in §2.5.** "Memory doesn't carry over on a new build" and
"the composed file has no `# Memory` section" are the same finding, not two.

### 2.7 A previously-accepted conclusion is now stale

`docs/analysis/TOKEN_TAX_FOLLOWUP_2026_07_04.md:16` records the
two-memory-systems question as **"Already resolved — Option A shipped"**:
`autoMemoryEnabled: false` written *"whenever the file's settings block is
(re)written"*, on the reasoning that *"AgentMux owns memory itself via
CLAUDE.md's global-brain + per-agent memory injection. The two-memory-systems
risk the doc flagged doesn't exist today."*

Both halves are no longer true:

- The guard is now scoped to **shared workdirs only** —
  `let workdir_is_shared = agent.working_directory.is_empty();`
  (`agent_open.rs:731`). Any agent with an explicit workdir keeps CLI
  auto-memory **on**. Verified live: no `autoMemoryEnabled` key exists in
  this agent's `.claude/settings.json`, and its memory directory holds a
  populated `MEMORY.md` plus five memory files.
- "AgentMux owns memory itself" is exactly backwards in practice (§2.5).
  AgentMux's memory is empty; CLI auto-memory (26 rows in
  `db_agent_native_memory`) is the only system doing real work.

The two memory systems **do** both run today, with their roles inverted
relative to the documented decision. That is the actual state to design from.

## 3. The three cases, re-assessed against the evidence

| Case | Reality | Verdict |
|---|---|---|
| **1. New agent** | System A composes an empty memory section (§2.5). Nothing seeds a System B scaffold either. | **Broken in effect** — a new agent gets no curated memory, and no structure to start writing one. |
| **2. Existing agent** | System A is rewritten every open, correctly — but with nothing in it (§2.5). System B persists and loads natively. | **Broken in effect for curated memory**; works for agent-authored memory, by the provider's doing rather than AgentMux's. |
| **3. After compaction** | Both systems are system-prompt-level and survive (§2.1, §2.2). | **Not broken.** The draft was wrong about this. |

**The real gap is not case 3 — it is that curated memory (System A) is empty
everywhere, and the two systems' roles are undocumented and inverted.**

## 4. Design

### 4.1 Make System A's emptiness visible (cases 1 + 2) — highest value

The single highest-value change: nothing anywhere reports that an agent is
running with no curated memory. It looks identical to working.

- Surface, per agent, whether its composed file actually contains a `# Memory`
  section, and whether any global-brain bundles exist in this channel.
- Because `db_bundles` is not globalized, a fresh build channel *always*
  starts with zero global brain (§2.5). That should be stated at the point
  the user would notice — not discovered by reading a 1089-byte file by hand.

### 4.2 Reconcile the two systems' roles, and write the decision down

§2.7's decision record is stale and actively misleading. This needs an
explicit, current answer to: *which system owns which kind of memory?*

Recommended, matching how they actually behave:

- **System B (CLI auto-memory)** owns agent-authored, session-derived facts.
  It already works, survives compaction, and has drift/versioning behind it.
  Leave it on; stop treating it as something to suppress.
- **System A** owns human-curated, cross-agent policy (global brain, Soul,
  AgentMD). Its job is to be *populated*, which today it never is.

Whatever is chosen, `TOKEN_TAX_FOLLOWUP_2026_07_04.md` must be corrected —
leaving a "resolved" marker on an inverted conclusion is how this went
unnoticed.

### 4.3 Surface memory in the startup payload (cases 1 + 2)

`buildStartupPayload` emits Identity, Description, Accounts, Startup
Instructions, Peer Agents — and **no memory of any kind**. Add a Memory
section: the memory directory path, the `MEMORY.md` index (index only, never
file bodies — it is one line per memory by contract), and an explicit "no
memories yet" when empty, so absence is distinguishable from the section
being dropped. Cap like Peer Agents already caps (10, then "…and N more").

This is worth doing independently of §4.1/§4.2: it is the only thing that
would tell an agent its memory directory exists at all.

### 4.4 Seed a memory scaffold for new agents (case 1)

Create the memory directory with a well-formed empty `MEMORY.md` index on
first materialization, so the first `MemoryWrite` has somewhere valid to land
and the Armory Memory tab has something to show. **No example memories** — a
fabricated memory is worse than none.

### 4.5 Post-compaction digest — reduced scope, and a recommendation

The repo owner selected "re-inject a memory digest turn" for case 3. That
choice was made against the draft's framing, which §2.1/§2.2 have since
disproven: **memory already survives compaction natively**, so a digest
re-stating the memory index would re-deliver something the model already has
in its system prompt — paying tokens and a visible turn per compaction for no
gain.

Recording this rather than silently building it. Options, in the order I'd
recommend them:

1. **Drop it.** Cases 1 and 2 are the real gaps; §4.1–§4.4 address them.
2. **Re-scope it** to what compaction *does* destroy — session-acquired
   working context, not memory files. That is a different feature
   (a work-state digest) and deserves its own spec rather than being
   retrofitted onto this one.
3. **Build it as specified**, accepting it is redundant with the provider's
   own behaviour.

If it is built, the mechanism is settled: the `compact_boundary` frame
(§2.4, Claude-only) via `handleSendMessage(…)` — the same path
`buildStartupPayload` already uses (`agent-view.tsx:2172`) — never the
`PreCompact` hook, which cannot emit output (§2.3), and never
`compaction_started`, which races the boundary
(`SPEC_COMPACTION_STARTED_RECONCILIATION_RACE_2026_09_02.md`).

### 4.6 Mid-session propagation (out of scope, flagged)

System A is written at **spawn only**. Editing Armory memory mid-session
never reaches the running agent — independent of compaction.
`TOKEN_TAX_ANALYSIS_2026_06_19.md` raised this and answered *"the logical
answer is no"* without resolving it. Still unresolved; not addressed here.

## 5. Non-goals

- **Not** changing `CLAUDE.md` ownership policy
  (`SPEC_CLAUDE_MD_OWNERSHIP_PROTECTION_2026_08_22.md`).
- **Not** merging Systems A and B. §2 shows they have genuinely different
  lifecycles and failure modes; §4.2 assigns roles rather than unifying.
- **Not** injecting memory *file bodies* anywhere. Index only.
- **Not** relevance ranking ("which memories matter now").
- **Not** changing compaction detection.
- **Not** seeding example memories (§4.4).
- **Not** solving mid-session propagation (§4.6).

## 6. Testing

- `buildStartupPayload`: memory section present with entries, explicit when
  empty, capped past the limit. Already a pure function with an existing test
  file.
- §4.1 indicator: correct for an agent with a populated `# Memory` section,
  for one without, and for a channel with zero global bundles.
- §4.4 scaffold: created once, well-formed, never overwrites an existing
  `MEMORY.md`.
- Regression guard for §2.5: assert the composed file contains a `# Memory`
  section when `content_map["memory"]` or a global bundle is non-empty. The
  absence of any such test is why an empty memory section shipped unnoticed
  across every agent.
- If §4.5 is built: fires once per real `compact_boundary`, never on
  `compaction_started`, suppressed when memory is empty.

## 7. Open questions

The draft's four unknowns are resolved (§2). What remains is a decision, not
an unknown:

1. **§4.5 — drop, re-scope, or build the post-compaction digest?** My
   recommendation is drop; the evidence says it is redundant.
2. **§4.2 — which system owns which memory?** Needs an explicit answer and a
   correction to `TOKEN_TAX_FOLLOWUP_2026_07_04.md`.
3. **Should a fresh build channel inherit global-brain bundles?** Today
   `db_bundles` is not globalized, so every new channel starts with an empty
   brain (§2.5). Everything else an agent carries — transcripts, native
   memory, definitions — *is* global (§2.6), which makes the global brain
   the lone exception and the direct root cause of "memory doesn't carry
   over" on a portable build. That is how this was noticed, and it is the
   single highest-leverage decision in this spec: globalizing `db_bundles`
   would fix cases 1 and 2 outright, without any of §4.3–§4.5.
