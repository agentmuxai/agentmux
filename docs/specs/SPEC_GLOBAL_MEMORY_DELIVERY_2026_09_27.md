# Spec: Global Memory delivery into agents — single source of truth

**Status:** active — delivery via the startup-instructions file shipped
(#1584, #2747, #2782, #2788, #2854, #3244). Hidden reinjection after compaction
and on a `fresh` session outcome shipped in #3502 and #3512, with the running
summary appended in #3673. P0 (§4: why a real `fresh` session got nothing) is
done, and P1 (reinjection re-delivers the startup file's own block, Operator
Config included) shipped with it in #3942. P2 — Claude's `SessionStart` hook
delivers the memory at every new session, `/clear` and compaction, with a notice
naming each entry's size — shipped in #3949 (srv side) and P2b (the hook, the
notice, and the fallback standing down when the hook delivered). Remaining:
live verification of P2 on a running instance, then P3 (retire the frontend
triggers); other providers (P4); edits reaching running agents (P5).
**Date:** 2026-09-27
**Author:** AgentA (agent, `~/.agentmux/agents/agenta-07017`), at the repo
owner's request: "lets consolidate all the docs … lets get a single source of
truth for this vector of work."
**Tracking issue:** #3925

This is the one doc for **how Global Memory gets into an agent's context**:
at launch, after compaction, and on a new or restarted session. It replaces
two docs that disagreed (§8.1) and indexes the shipped specs it builds on.
Anything about how Global Memory is **stored, edited, shown in the Armory, or
shared across instances** is out of scope and stays in its own docs (§8.2).

Code is cited by file and symbol, not line, because lines drift. Verified
against `main` @ `af2080e9e` (2026-09-27).

---

## 1. Goal (owner direction)

> when an agent is opened into a new session, and after compaction, the global
> memory needs to be reinjected with only a message to the user.
> — repo owner, 2026-09-27

The earlier ask that started the reinjection work:

> the global/personal memory needs to be reinjected after every compression.
> when injecting, put just a label, but essentially the agent has to read it
> all. … we also want the token count of each. — repo owner, 2026-09-22

The same session settled the cost objection:

> we have no choice, the memory is critical.

So the requirement has three parts:

1. **When:** every new session and every compaction.
2. **What:** the full Global Memory content, not an index.
3. **What the user sees:** a short notice, not the content.

## 2. Vocabulary

| Tier | What it is | Store | Written by |
|---|---|---|---|
| **Operator Config** | AgentMux-maintained Global Memory entries (e.g. "App API", "Environment & Gotchas") | `db_bundles`, `is_global=1, is_system=1` | seeded from `agentmux-srv/operator-config-seed.json` at every startup (`operator_config_seed.rs`), unless a human edited the row |
| **Global Memory** | Workspace-wide notes, shared by every agent in the workspace | `db_bundles`, `is_global=1, is_system=0` | humans (Armory) and agents (`GlobalMemoryWrite`) |
| **Personal Memory** | One agent's own provider-native memory files (e.g. Claude's `projects/<dir>/memory/*.md`) | provider files + `db_agent_native_memory` mirror | the agent |
| **Provider Config** | The provider's own global config (e.g. `~/.claude/CLAUDE.md`) | the host | the user; isolated from spawned agents (#2854) |

"Global Memory" in this doc means Operator Config + Global Memory together
unless a row says otherwise. Personal Memory appears only where the
reinjection path already sends it (§3.3).

## 3. What exists today

### 3.1 Storage and write API (shipped, unchanged by this doc)

- Entries are `db_bundles` rows in the channel's store; `Store::bundle_list_global`
  (`storage/bundles.rs`) lists system rows first, then by `sort_order`, then name.
- Agents write through the `GlobalMemory*` MCP tools (`agentmux-mcp/src/main.rs`),
  which go through `global_memory_write_impl` (`server/app_api/mod.rs`). Writes are
  capped at 1 MB and refuse system rows. Every write publishes `memories:changed`.
- History: append-only `db_bundle_versions` (history/diff/revert, #3448), and a
  write-through record in the global transcript store (`global_memory_record.rs`,
  #3810).

### 3.2 Delivery at launch: the startup-instructions file (shipped)

**Composition.** `format_global_bundle_block` (`storage/bundles.rs`) writes system
rows as `# [AgentMux System] <name>` under a "HIGHEST PRIORITY … OVERRIDE"
preamble, then ordinary rows as `# [Workspace] <name>`, separated by `---`
(#2782). The block goes under the file's `# Memory` heading.

**Two write paths, both at open/launch time only:**

- *Launch from the UI:* `agent-model.ts` → `buildConfigFiles`
  (`agent-config-builder.ts`) → `WriteAgentConfigCommand` → `editor_handlers.rs`,
  where `inject_global_bundles` inserts the block.
- *`agent.open` RPC:* `write_agent_config_files` (`server/app_api/agent_open.rs`)
  puts the block in front of the memory content, then `build_config_files`
  (`agent_config.rs`) runs.

**Per-provider file** (`ProviderConfig::startup_instructions_filename`,
`providers.rs`, #2788):

| Provider | File |
|---|---|
| claude, muxcode | `CLAUDE.md` |
| codex, openclaw, copilot | `AGENTS.md` |
| gemini, antigravity | `GEMINI.md` |
| qwen | `QWEN.md` |
| pi | `.pi/APPEND_SYSTEM.md` |
| kimi | **none: Kimi gets no Global Memory** |

**Existing files.**

- `CLAUDE.md` uses `write_claude_md_respecting_ownership` (#2747). If the user
  owns the file, AgentMux writes `.claude/AGENTMUX_MEMORY.md` instead and appends
  one `@.claude/AGENTMUX_MEMORY.md` import line.
- Every other file uses `write_startup_instructions_respecting_existing`, which
  only checks whether the file exists. **If the user already has an `AGENTS.md`
  or `GEMINI.md`, Global Memory is silently not delivered.** There is no
  side-file fallback.

**Why this reaches the model.** The provider reads the file into its system
prompt at process start, and Claude rebuilds that prompt on every request, so
compaction does not remove it (measured: `SPEC_MEMORY_CARRYOVER_LOAD_AND_MANAGE_2026_09_05.md`
§2.1–2.2). A respawn inside a pane re-reads whatever the last launch wrote.

**When the file is not rewritten:** on a respawn inside a pane (crash,
resume-retry, eager resume), and on a mid-session memory edit. `memories:changed`
only refreshes the Armory.

### 3.3 Hidden reinjection (shipped #3502 + #3512, Claude only)

The model gets the full text as a real user turn; the pane shows only a label.

- **Triggers**, both in the frontend (`useAgentStream.ts`):
  - a live `system`/`compact_boundary` frame → `trigger(…, "compaction")`;
  - an `agentmux_session_outcome` with `outcome === "fresh"` →
    `trigger(…, "fresh_session")`.
- **Controller** (`memory-reinjection-controller.ts`): guards against firing
  twice at once, and defers while a turn is running.
- **Content** (`memory-reinjection-fetch.ts`, `memory-reinjection.ts`
  `composeReinjectionMessage`):
  - Global Memory as the startup file's own sections (`globalmemory:sections`,
    `global_bundle_sections` in `storage/bundles.rs`), **Operator Config
    included**. Joined by `GLOBAL_SECTION_SEPARATOR` they are the block
    `format_global_bundle_block` writes, byte for byte (P1). Before P1 this
    filtered to `is_global && !is_system` and dropped the headings;
  - plus Personal Memory files (without the `MEMORY.md` index), per D9;
  - wrapped in a system-reminder block that opens with the fixed sentence
    "Your memory was reinjected because your working context was just reset."
- **Send:** `AgentInputCommand` with `hidden: true`. The server handler
  (`agent_handlers/input.rs`):
  - sets `HIDDEN_REINJECTION_BLOCKS` (`app_api/session.rs`) so the digest, title
    and next-prompt readers skip the turn;
  - marks the turn `TurnOrigin::System`;
  - for compaction only, appends AgentMux's running summary
    (`continuity_state::with_state_after_compaction`, #3673).
- **What the user sees:** a `MemoryReinjectionNode` (`types.ts`) rendered by
  `DocumentRow.tsx` as "🧠 Memory reinjected — N global, M personal (~T tok, est.)".
  Hovering shows a per-entry token breakdown and compress/delegate advice once
  the size band is high. History replay recognizes the signature
  (`stream-parser.ts`, `parseHistoryLines.ts`, Rust `is_hidden_reinjection_text`).

### 3.4 When a `fresh` outcome is emitted

`fresh_start_needs_disclosure` (`blockcontroller/persistent/mod.rs`) returns
`attempted_resume_sid.is_none() && generation == 1`, and only fires when there
is a prior transcript. The resume-retry path emits its own
(`resume_retry.rs`). A fresh session onto prior history also carries AgentMux's
continuation packet (`carry_continuation`, #3643), so it gets both the packet and
the memory reinjection.

### 3.5 Hooks AgentMux installs

Only `PreToolUse(Bash)` and `PreCompact` (`agentmux-bashwrap precompact`, a
"compaction started" ping) are installed (`agent_config.rs`,
`agent-config-builder.ts`). **There is no `SessionStart` hook.** (The
`SessionStart` matches in `frontend/app/store/agent-document/` are the
reducer's own session events, not Claude Code hooks.)

## 4. Coverage today, and what has been verified

| Case | Global Memory reaches the model via | User sees | Verified live? |
|---|---|---|---|
| Brand-new agent / new session | startup file; the SessionStart hook adds it only when no loaded startup file carries it (Claude, LC3 of SPEC_LAUNCH_CONTEXT_WORKSPACE_RULE_AND_STARTUP_FILES_2026_09_30.md) | the "Given to the agent" card: startup files, then memory (LC2) | yes |
| Normal resume | startup file (rewritten at open) | nothing | yes |
| Resume refused or failed → `fresh` | startup file + hidden reinjection | label | the trigger fired, but it had nothing to send before P1 (below); not yet seen after P1 |
| After compaction (Claude) | startup file + hidden reinjection + summary | label | no: no compaction observed since #3502 |
| After compaction (other providers) | startup file only | a compaction node, no reinjection | — |
| Kimi, any case | nothing | nothing | — |

**Live check, 2026-09-26/27.** AgentA's restart onto 0.57.6 (which includes
#3512) at 12:49:50Z was exactly the `fresh` case:
- the srv logged `resume gate: … can't resume; starting fresh with the record`
  and `carrying AgentMux's record`;
- no transcript on the host since 2026-09-22 contains the reinjection signature
  sentence, including that session (`61a700ea…`).

So the fresh-session reinjection never reached the model there.

**Why (P0, 2026-09-27).** The plumbing worked; the suppression rule stopped it.
- srv emitted the disclosure: `spawned with no --resume while prior history
  exists — disclosing a fresh start` at 12:49:50Z, so the pane received a `fresh`
  outcome.
- The pane was busy with that session's first turn, so the controller deferred,
  then fetched after the turn ended.
- The fetch read `listmemories` from `id_store` (`identity-store.db`), the same
  store the startup file is composed from. That channel held **only the two
  Operator Config rows**, which the fetch filtered out (`!is_system`).
- Personal Memory was empty: the login was new at 12:49.
- With no entries, `shouldReinject` returned false and nothing was sent,
  silently.

So G3 was the cause: the only memory present was the tier reinjection excluded.
P1 fixes it.

## 5. Decisions of record

| # | Decision | Source |
|---|---|---|
| D1 | Global Memory is composed into the provider's startup file at launch; system rows come first with an override preamble | #2782, `SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md` |
| D2 | Never overwrite a user-owned `CLAUDE.md`; use a side file + `@import` | #2747, `SPEC_CLAUDE_MD_OWNERSHIP_PROTECTION_2026_08_22.md` |
| D3 | One startup filename per provider; Kimi has none | #2788, `SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md` |
| D4 | Reinject after compaction despite the redundancy argument: "availability in the system prompt is not the same as the model attending to it" | owner, 2026-09-22; overrides carry-over §4.5 |
| D5 | Reinject **full bodies**, not an index (reverses carry-over §5's "index only" non-goal for reinjection) | reinjection spec §1.1 |
| D6 | The user sees a label only; content goes to the model as a hidden turn | owner, 2026-09-22 |
| D7 | No size cap; a graduated warning with compress/delegate advice instead | reinjection spec §3.4, §7 Q2 |
| D8 | Reinject on every new session too, not just compaction and `fresh` | **owner, 2026-09-27: new, not yet built** |
| D9 | Keep reinjecting Personal Memory alongside Global Memory (Q3) | owner, 2026-09-27 |
| D10 | No reinjection on a normal resume (Q4): the conversation is intact and the startup file still carries the memory | owner, 2026-09-27 |
| D11 | The notice shows the size of each injection: total tokens, plus each entry's bytes and tokens | owner, 2026-09-27; shipped for the current path (the label and its hover breakdown), and P2's delivery event must carry it too |

## 6. Gaps

- **G1 — resolved for Claude (P2):** the `SessionStart` hook delivers at every
  `startup` and `clear`, not only on a `fresh` outcome. A normal resume is
  deliberately excluded (D10). Not yet verified on a running instance.
- **G2 — resolved (P0):** the `fresh` path did fire; it had nothing to send
  because of G3 (§4).
- **G3 — resolved (P1):** reinjection now re-delivers the startup file's own
  block, Operator Config and the `[AgentMux System]`/`[Workspace]` headings and
  preamble included.
- **G4 — resolved for Claude (P2):** the hook runs inside the CLI, pane or no
  pane. The frontend trigger remains as a fallback that asks srv first
  (`memorydelivery:claim_fallback`) and stands down when the hook delivered.
- **G5 — Claude only:** no compaction signal from other providers. The
  token-drop heuristic draws a node but does not reinject.
- **G6 — delivery holes in the startup file:**
  - Kimi gets nothing;
  - a user-owned `AGENTS.md`/`GEMINI.md` silently blocks delivery (no side-file
    fallback like D2).
- **G7 — edits don't reach running agents:** a Global Memory edit, Operator
  Config included since P1, reaches the model at the next launch, compaction or
  `fresh` reinjection. A respawn inside a pane re-reads the old startup file.
- **G8 — resolved (D9):** Personal Memory stays in the same hidden turn.
- **G9 — never built:**
  - the opt-out setting (reinjection spec §7 Q1);
  - the `WorkEnqueue` delegation action (§3.4.3);
  - carry-over §4.3 (a Memory section in `buildStartupPayload`) and §4.4 (seed
    an empty memory index).
- **G10 — double carry on fresh:** the continuation packet (#3643) and the
  memory reinjection both arrive on a fresh session. Nobody has checked for
  overlap or ordering problems.
- **G11 — seeded default Global Memory never reaches agents:** found during P0.
  - `agentmux-srv/agent-seed.json` seeds two global entries, "Agent Memory" and
    "Workspace Rules" (the second described as "injected into all agents").
  - `agent_seed::seed_agents` writes them to `mstore` (`objects.db`), but Global
    Memory has been read from `id_store` (`identity-store.db`) since at least
    #3287, both for the startup file and for reinjection.
  - So neither the startup file nor reinjection carries them. Whether they
    should reach every agent is an owner call before anyone moves them.

## 7. Plan

P0–P2 are done (P2 awaits live verification); P3–P5 not started.

- **P0 — explain G2. Done:** see §4. The `fresh` trigger fired, and G3 left it
  nothing to send. G4's double-fire risk moves to P2's claim design.

- **P1 — one composition for every path (G3). Done.**
  - `global_bundle_sections` (`storage/bundles.rs`) renders the Global Memory
    block as sections. `format_global_bundle_block` is exactly those sections
    joined by `GLOBAL_SECTION_SEPARATOR`, so the startup file's output is
    unchanged (pinned by `global_sections_join_to_the_startup_block_exactly`).
  - `globalmemory:sections` serves them from `id_store`, the startup-file
    writers' store. The frontend reinjection (`memory-reinjection-fetch.ts`) uses
    them, so the Global section of a reinjection is the startup block verbatim,
    Operator Config included.
  - Each section's `size_bytes` is what it adds to an injection, and feeds the
    notice's per-entry breakdown (D11). Personal Memory is unchanged (D9).
  - History replay now ends a section at the next `# Global/Personal Memory (N
    entries)` header rather than at any `# ` line, which the sections'
    own headings would otherwise trip.
  - P2's hook will read the same endpoint.

- **P2 — move the trigger into the backend with a Claude `SessionStart` hook.**
  - Claude Code runs `SessionStart` with `source` = `startup` | `resume` |
    `clear` | `compact` and adds the hook's `additionalContext` to the model's
    context without showing it as a conversation turn. One hook covers new
    session, `/clear` and compaction (G1, G4). It does nothing for
    `source=resume` (D10).
  - Install it next to the existing `PreCompact` hook (`agentmux-bashwrap`
    subcommand; same install sites as §3.5).
  - The hook fetches the block from P1's endpoint and emits it as
    `additionalContext`.
  - **Keep the running summary on compaction.** Today it is appended only in
    `agent_handlers/input.rs`, via `continuity_state::with_state_after_compaction`
    (#3673). When `source=compact`, the endpoint must append that summary the
    same way. Otherwise retiring the hidden turn drops the backstop against
    commitments the provider's own summary left out.
  - **Only one path per event.** Delivery is an **atomic claim** in srv, keyed
    by (session, reason, compaction boundary). A lookup of completed deliveries
    is not enough: on `/compact` the frontend sees the boundary while the hook
    is still running, so both would see "not yet delivered".
    - the hook claims when it fetches (state `pending`);
    - the frontend fallback asks to claim **before** it hides output or
      dispatches `TurnStart`. If the claim is `pending`, it waits for the hook to
      acknowledge or time out. If the claim is already taken, srv answers
      `skipped` and the controller stays idle;
    - a skip must never be a silent accept. The controller sets `hiding` and a
      pending node before `sendRpc`, and clears them only on a rejected RPC or
      the hidden turn's `session_end`, so a silent drop would leave the pane
      busy and suppressing output.
    - a claim becomes a delivery only once the hook **acknowledges** it: after
      writing and flushing valid `additionalContext`, the hook calls srv back. A
      `pending` claim with no acknowledgement (the hook timed out, crashed or
      wrote bad output) expires, and the waiting fallback takes it over.
    Without this gate, every compaction injects the memory twice.
  - **The notice ships with the hook (D6).**
    - srv emits one event per delivery (reason, entry names, and each entry's
      bytes and tokens: D11);
    - the pane renders the existing `MemoryReinjectionNode` from that event, not
      from the frontend's hidden turn.
    - the event is **persisted** to the block's durable history with a stable
      id (the delivery record's), so a pane that mounts later, or a history
      replay, shows the notice once. Background agents have no pane at delivery
      time, and after P3 no hidden turn remains in the transcript for
      `parseHistoryLines.ts` to rebuild it from.
    Otherwise every hook delivery would be silent, because today the label is
    produced only when the frontend's own hidden turn ends.
  - **Measured, 2026-09-27** (the CLI AgentMux bundles, 2.1.280, headless
    `claude -p --settings <file>` with test hooks):
    - `additionalContext` reaches the model: a code word placed in it was
      answered.
    - **About 10,000 characters per hook command.** 9,950 arrived whole. At
      10,050 and above the CLI replaced it with a saved file plus a **2 KB
      preview** and the file's path, which the model could not read.
    - **The cap is per hook command, and many are allowed.** Six hooks of
      9 KB each (about 54 KB) all arrived whole.
    - **Order follows completion, not configuration.** The hooks run in
      parallel, and a staggered test came back reversed.
    - `source` has a fifth value, **`fork`** (a `--fork-session` resume, which
      is how AgentMux resumes a relocated session). Like `resume`, it continues
      an intact conversation, so it gets no delivery (D10).
    - Not yet verified: whether `compact` fires after an auto-compaction as
      well as after `/compact`.
  - **So the memory is split:** parts of at most 9,000 characters, each labelled
    "[AgentMux memory — part N of M]", and a fixed number of hook commands
    (8, so up to 72,000 characters). Each command asks srv for its own part
    number. A larger memory is cut at the end, and the last part says how much
    was left out.
  - **Built in two PRs:**
    - **P2a, srv side, inert until P2b installs the hook:**
      - `backend/memory_delivery.rs`: composition, splitting and the
        source rules;
      - `server/memory_delivery_handlers.rs`:
        `POST /api/v1/agent/memory/session-start/part` composes once per
        (block, session, reason) and serves each part from that cache, so
        parallel hooks get consistent parts;
        `…/session-start/ack` marks a part written. Once every part is
        acknowledged, one persisted `agentmux_memory_injected` frame goes to
        the pane with each entry's label, bytes and tokens (D11);
      - `continuity_state::running_summary_section` supplies the summary on
        compaction, formatted as the hidden reinjection formats it.
    - **P2b, switched on:**
      - `agentmux-bashwrap sessionstart --part N` (`sessionstart.rs`): reads
        the hook's stdin, asks for its part (3 s timeout), prints it as
        `additionalContext`, then acknowledges it. Any failure prints nothing
        and exits 0, so the session starts regardless.
      - The hook entry: one `SessionStart` entry, no matcher, 8 commands,
        installed by both builders (`agent_config.rs`
        `agentmux_sessionstart_entry`, `agent-config-builder.ts`, whose
        `SESSION_START_HOOK_PARTS` is asserted against Rust's `HOOK_PARTS`). A
        user's own `SessionStart` hooks are kept on both merge paths, before
        ours; without that, a settings.json one would have been dropped.
      - The notice: `memory-injected.ts` turns the frame into the existing
        `MemoryReinjectionNode`, live (`useAgentStream.ts`) and on replay
        (`parseHistoryLines.ts`), keyed by the frame's own id.
      - One delivery per event. `memorydelivery:claim_fallback` is an atomic
        claim in srv, keyed by (block, reason):
        - the hidden-reinjection controller asks right before it would hide
          anything, and only when it is really about to fire (never while
          deferred);
        - the hook's parts come back empty once the fallback has claimed;
        - the fallback stands down once the hook has delivered;
        - a hook delivery still in flight gets up to 4 s, then the fallback
          delivers and the hook's unfinished delivery is closed, so a late
          acknowledgement adds no second notice;
        - a failed claim means deliver: a duplicate beats no memory.
      - Semantics are at-least-once: a hook whose acknowledgement is lost after
        Claude read its output can still lead to the fallback delivering again
        (the Codex note on #3925).

- **P3 — retire the frontend triggers for Claude.** Only after P2 is verified
  live: the hook fires for `startup`, `clear` and `compact`, and never delivers
  on `resume` (D10); the running summary arrives on compaction; the notice appears every time; and
  nothing was injected twice. Keep the hidden-turn primitive: P5 needs it.

- **P4 — providers other than Claude (G5, G6).**
  - Use each provider's equivalent hook where one exists; otherwise the startup
    file stays the only path, and the pane says so once.
  - Add a side-file fallback for user-owned `AGENTS.md`/`GEMINI.md`.
  - Decide what Kimi gets.

- **P5 — edits while running (G7).**
  - On `memories:changed`, mark running agents stale.
  - Reinject at their next turn boundary (the hook path can't do this; it needs
    the hidden-turn primitive from §3.3).

## 8. Doc and issue map

### 8.1 Replaced by this doc

| Doc | Now |
|---|---|
| `SPEC_HIDDEN_MEMORY_REINJECTION_AFTER_COMPACTION_2026_09_22.md` | `superseded`. It is still the detailed design reference for the shipped hidden-turn primitive, label node and size bands; its open items moved here (G9, §7) |
| `SPEC_MEMORY_CARRYOVER_LOAD_AND_MANAGE_2026_09_05.md` | `superseded`. Its measurements (§2) still stand; its recommendation against reinjecting was overridden (D4) |

### 8.2 Still authoritative for their own scope (indexed, not replaced)

- **Delivery building blocks, all implemented:**
  - `SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md`
  - `SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md`
  - `SPEC_CLAUDE_MD_OWNERSHIP_PROTECTION_2026_08_22.md`
  - `SPEC_ISOLATE_HOST_CLAUDE_MD_2026_08_31.md`
  - `SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md`
  - `SPEC_SYSTEM_TIER_GLOBAL_MEMORY_SEEDING_2026_09_15.md`
- **Write side:**
  - `SPEC_AGENT_FACING_GLOBAL_MEMORY_API_2026_09_15.md`
  - `SPEC_GLOBAL_MEMORY_UNIFY_SYSTEM_AND_ORDINARY_2026_09_15.md`
  - `SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md`
- **Compaction signal:** `SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md`.
- **Conversation continuity** (packet, running summary):
  `SPEC_DURABLE_CONVERSATION_MEMORY_2026_09_23.md`,
  `SPEC_RESUME_GATE_AND_SAME_IDENTITY_CONTINUATION_2026_09_25.md`.
- **Adjacent vectors, not this one:**
  - cross-instance sync: `SPEC_CROSS_INSTANCE_GLOBAL_MEMORY_SYNC_2026_09_20.md`, #3477;
  - memory following an agent across accounts/channels: `SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md`;
  - export/import: `SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md`, #3148;
  - the Armory Memory UI specs;
  - `docs/status/STATUS_ARMORY_MEMORY_CONSOLIDATION_2026_09_09.md`.

### 8.3 Issues

- **Tracking issue for this doc's gaps and plan:** #3925. It is the only
  issue for this vector.
- **Related, separate:** #3477 (Global Memory writes invisible to another
  instance) and #3148 (portability tracking).
- No other open issue covers delivery or reinjection; none were found closed
  without being fixed.

## 9. Open questions for the owner

- **Q1 — opt-out:** is reinjection always on, or opt-out per agent or workspace?
  (carried from the reinjection spec §7 Q1)
- **Q2 — expandable notice:** should the notice expand to show what was injected,
  for the operator's debugging? (reinjection spec §7 Q4)
- **Q3 — answered (D9):** keep reinjecting Personal Memory alongside Global
  Memory.
- **Q4 — answered (D10):** no reinjection on a normal resume.
- **Q5 — the delegation action:** should `WorkEnqueue` ever auto-fire, or always
  ask? (reinjection spec §3.4.4)

## 10. History

| Date | PR | What |
|---|---|---|
| 04-16 | #408, #413 | Structured startup sequence |
| 06-19 | #1584 | First Global Memory bundles |
| 08-22 | #2734, #2747 | Global/Personal naming; CLAUDE.md ownership protection |
| 08-24 | #2782, #2788 | System tier + composition order; per-provider startup files |
| 08-31 | #2854 | Host `~/.claude/CLAUDE.md` isolated from agents |
| 09-06 | #3011 | Carry-over analysis: compaction keeps the startup file; argued against reinjecting |
| 09-15 | #3237, #3244 | Agent-facing Global Memory API; Operator Config seeding |
| 09-20 | #3448 | Global Memory history/diff/revert |
| 09-22 | #3502, #3512 | Hidden reinjection after compaction; also on a `fresh` outcome |
| 09-24 | #3643, #3673 | Continuation packet on fresh sessions; running summary appended after compaction |
| 09-25–26 | #3810, #3811, #3812 | Global Memory's own record; import into isolated channels; bundle id follows the agent |
| 09-27 | #3926 | Consolidation; D8 (new sessions); live check finds G2 |
| 09-27 | #3942 | P0 (the fresh path had nothing to send: G3) and P1 (reinjection = the startup block, Operator Config included); D9–D11; G11 |
