# AgentMux Context Analysis
**Date:** 2026-06-19 | **Version:** v0.46.5

---

## 1. CLI Baseline (what `claude -p` uses before AgentMux adds anything)

Measured in an empty directory, no CLAUDE.md, no MCP servers.
Source: claudecodecamp.com system prompt teardown + GitHub issue #52979.

| Component | Tokens | Controlled by |
|---|---|---|
| System prompt text (instructions, output rules, tone) | ~2,300–3,600 | Anthropic |
| Built-in tool definitions (Bash, Edit, Read, Write, etc.) | ~14,000–17,600 | Anthropic |
| Memory preamble (hardcoded instructional template for MEMORY.md) | ~11,300 | Anthropic |
| **Total baseline** | **~27,000–32,000** | **Anthropic** |

AgentMux has no access to and cannot modify any of this.

The system prompt and tool definitions are cache-eligible. For OAuth subscription users
the cache TTL is **1 hour** — so turns 2+ within a session pay ~10% of the baseline
cost for these layers, not the full amount.

---

## 2. What AgentMux Adds

| Component | Tokens | When |
|---|---|---|
| CLAUDE.md (default, no customization) | ~28 | Fresh session only |
| CLAUDE.md (with soul + agentmd + memory + skills) | 28 – unbounded | Fresh session only |
| Startup payload (identity + accounts + peers + checklist) | ~46–548 | Turn 1 of new session only |

**AgentMux's direct addition to the baseline is ~28 tokens by default.**

Both components are injected as **user messages**, not system prompt. They are part
of the conversation history — sent once, then carried forward by `--resume`.

The startup payload is the one item worth scrutiny: it is ~487 tokens at typical
configuration, and the STARTUP verification checklist inside it forces a full
agentic verification turn before the user gets any help (see §4).

---

## 3. How MEMORY.md Actually Works

This section covers only verified facts from official docs and confirmed GitHub issues.

### What it is

Claude Code's native cross-session memory. Claude writes to it autonomously (build
commands discovered, patterns noticed, mistakes to avoid). Users can also edit it
directly.

**Path:** `~/.claude/projects/<git-repo-root-hash>/memory/MEMORY.md`

All worktrees of the same git repo share one memory directory.

### When it loads

MEMORY.md is injected **once at fresh session start** as a user message (same
mechanism as CLAUDE.md — wrapped in `<system-reminder>` XML tags in the first user
message of the session).

**`--resume` does NOT re-inject MEMORY.md.** Source: official sessions docs —
resume "reopens it under the same session ID and appends new messages." It is not
a new session start. The MEMORY.md content from session start is already in
conversation history and is replayed via `--resume` like any other message.

**After `/compact`:** MEMORY.md is re-read from disk and re-injected. Compaction
clears conversation history and re-loads all project context files.

### Size limit

The first **200 lines or 25KB** of MEMORY.md (whichever comes first) loads at
session start. Content beyond this threshold is silently ignored. Topic files
(`memory/debugging.md`, etc.) do **not** load at startup — only on demand.

### The memory preamble bug

Setting `autoMemoryEnabled: false` in `.claude/settings.json` suppresses Claude's
write attempts but does **not** suppress the hardcoded memory preamble template.
The ~11,300-token preamble (on Sonnet) is still injected at session start even
when memory is disabled.

Source: GitHub issue #63903 (open, 2026-05-30). Previous issue #44829 was closed
as "not planned."

This means: **there is currently no way to avoid the 11,300-token memory preamble.**
It is Anthropic-controlled overhead, part of the ~27–32K baseline.

### Disabling auto-memory writes

```json
// .claude/settings.json (project or user scope)
{ "autoMemoryEnabled": false }
```

Or: `export CLAUDE_CODE_DISABLE_AUTO_MEMORY=1`

This stops Claude from writing new memories. It does not suppress the preamble.

---

## 4. The Interaction Between AgentMux and MEMORY.md

### Two separate memory systems are running simultaneously

| System | Writes | Loads | Path |
|---|---|---|---|
| CLI auto-memory | Claude decides autonomously | Fresh session + post-compact | `~/.claude/projects/<hash>/memory/MEMORY.md` |
| AgentMux memory | AgentMux on agent definition save | Every spawn (reads CLAUDE.md from disk) | `<workdir>/CLAUDE.md` (memory section) |

**These are additive.** Both are injected at session start. The same fact (e.g., "user
prefers patch bumps") could end up in both, paying the token cost twice.

### Does AgentMux's per-spawn model pick up CLAUDE.md changes mid-session?

Unclear from documentation alone. The logical answer is **no**:

- CLAUDE.md is injected as a user message on fresh session start
- That message is stored in the JSONL transcript
- `--resume` replays the transcript including the original CLAUDE.md injection
- The CLI has no reason to re-read CLAUDE.md from disk on every `--resume` spawn

If this is correct, memory written to CLAUDE.md by AgentMux mid-session is **not**
visible to the agent until the next fresh session or after `/compact`. Needs
verification by inspecting actual `claude -p --resume` behavior with a modified
CLAUDE.md between spawns.

### What this means for AgentMux memory strategy

If the agent already "knows" the memory content from session start (it's in
conversation history), writing it again into CLAUDE.md is redundant for the
current session. CLAUDE.md memory's value is **cross-session persistence** only —
so a fresh start or post-compact session picks up what was learned.

---

## 5. Context Window: Where It Actually Goes

For a resumed session (turn N), the full API request contains:

```
[CACHED — 1hr TTL for OAuth users]
  system prompt: ~27,000–32,000 tokens   ← Anthropic, not AgentMux
    ↳ core instructions: ~2,300–3,600
    ↳ tool definitions: ~14,000–17,600
    ↳ memory preamble: ~11,300

[IN CONVERSATION HISTORY — grows with session]
  message 1 (synthetic, session start):
    CLAUDE.md: ~28 tokens default         ← AgentMux writes, CLI injects
    MEMORY.md: 0 → 200 lines max          ← CLI writes and injects
  message 2 (turn 1):
    AgentMux startup payload: ~487 tokens ← AgentMux, new sessions only
    [agent's startup verification response if STARTUP checklist is set]
  messages 3..N: all prior turns
  message N+1: current user message
```

The dominant growing cost is **conversation history**, not any injection.
At ~2,000 tokens per exchange, a 50-turn session adds ~100,000 tokens of history.
The CLI auto-compacts when the window fills (~200K total), producing a summary at
~12% of the original size.

---

## 6. Optimizations

### P1 — Default startup to `__SKIP__`

The STARTUP verification checklist (~317 tokens + ACTION REQUIRED directive) forces
an agentic verification turn on every new session. For OAuth users this wastes a quota
turn before the user gets any help. The `/startup` slash command already exists as the
opt-in path.

```js
// scripts/gen-seed.js
content: {
    env: `AGENT_NAME=${d.id}\nAGENTMUX_AGENT_ID=${d.name}`,
    startup: "__SKIP__",
},
```

### P2 — Audit the two-memory situation

Determine whether CLI auto-memory (`MEMORY.md`) and AgentMux memory (in `CLAUDE.md`)
are accumulating duplicate facts. Options:

- **Option A:** Disable CLI auto-memory via AgentMux's `.claude/settings.json` write
  (`autoMemoryEnabled: false`) and let AgentMux own all memory. Downside: loses
  Claude's autonomous memory writes; can't suppress the 11,300-token preamble anyway.
- **Option B:** Let CLI auto-memory run and remove the `memory` section from AgentMux's
  CLAUDE.md write. Let Claude write its own cross-session memory natively. Simpler.
- **Option C:** Keep both, but scope them: CLI auto-memory for technical/task facts
  (build commands, patterns); AgentMux memory for user-level preferences and
  identity context.

### P3 — Verify CLAUDE.md re-read behavior on `--resume`

Confirm empirically whether `claude -p --resume <id>` re-reads CLAUDE.md from disk or
uses the version already in conversation history. The answer determines whether
mid-session memory writes in AgentMux are visible on the next turn.

Test: write a sentinel string to CLAUDE.md after turn 1, then on turn 2 ask the agent
what CLAUDE.md says. If it sees the sentinel, --resume re-reads from disk. If not, it
uses history.

### P4 — Add `--exclude-dynamic-system-prompt-sections`

This CLI flag moves per-machine content (working directory, OS, git info) from the
system prompt into the first user message. Designed for multi-user scripted workloads.
Improves cache reuse when multiple machines run the same agent definition.

Add to `agent_handlers.rs:3031` default CLI args.

---

## Sources

All facts verified against primary sources:

- [Claude Code memory docs](https://code.claude.com/docs/en/memory)
- [Claude Code sessions docs](https://code.claude.com/docs/en/sessions)
- [Claude Code how-it-works docs](https://code.claude.com/docs/en/how-claude-code-works)
- [GitHub #63903 — autoMemoryEnabled=false doesn't suppress preamble](https://github.com/anthropics/claude-code/issues/63903)
- [GitHub #52979 — 27–31K tokens for trivial prompts](https://github.com/anthropics/claude-code/issues/52979)
- [GitHub #45188 — 70K token growth in 5 days](https://github.com/anthropics/claude-code/issues/45188)
- [claudecodecamp.com — system prompt teardown](https://www.claudecodecamp.com/p/inside-claude-code-s-system-prompt)
- [justacuriousengineer.substack.com — API traffic analysis](https://justacuriousengineer.substack.com/p/breaking-down-claude-codes-prompt)
- `agentmux-srv/src/backend/agent_config.rs`
- `agentmux-srv/src/backend/blockcontroller/subprocess.rs`
- `frontend/app/view/agent/startup/buildStartupPayload.ts`
- `agentmux-srv/agent-seed.json`
