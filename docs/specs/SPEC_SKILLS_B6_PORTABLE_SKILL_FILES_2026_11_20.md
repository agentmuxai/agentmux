# Spec: Skills B6, portable skill files for non-Claude agents

**Status:** implemented, 2026-11-20, in #PRNUMBER

Follows the skills spec (#8050), which is not on `origin/main` (checked at 0.60.35, commit 1c35884: no document under `docs/specs` defines item B6). This defines the missing part, written from the code as it is today.

## 1. The requirement

A skill in the catalog (`Skills` tool, Memory > Skills) reaches an agent as guidance the agent loads when its description fits. Claude Code does that itself. A non-Claude agent (Codex, Gemini, Qwen, dsh, Kimi and the other providers) cannot, so while skills do not reach it, the long-form text stays in its startup context (Global Memory entries that were slimmed to "load the `x` skill" still have to be carried in full for it). B6 makes a skill reach a non-Claude agent as a file it reads on demand, so that long-form text can go.

## 2. What exists today (read from `origin/main`)

- `build_config_files` (`crates/srv-core/src/backend/agent_config.rs`) writes every effective skill to `.claude/skills/<slug>/SKILL.md` (an `agent-skill`) or `.claude/commands/<trigger>.md` (a `prompt`). It does this **for every provider**, not only Claude: a Codex agent's folder already holds the files.
- `skills_index_text` puts a "# Available Skills" index in the startup file. For Claude it lists only skills without a file of their own (so, in practice, none). For every other provider it lists all skills as `**name** (trigger: /x) — description` under the line "Use `/<trigger>` to invoke a skill."
- `agent_skill_files::refresh_for_block` rewrites the skill files at each controller start; `managed_skill_file_paths` limits what it touches to `.claude/commands/` and `.claude/skills/`.
- `computer_use_skill::sibling_files` writes the files beside a computer-use skill.

## 3. The gap

For a non-Claude agent the index says "Use `/<trigger>` to invoke a skill". There is no slash command in that CLI, and an `agent-skill` has no trigger at all in most cases. Nothing tells the agent that `.claude/skills/<slug>/SKILL.md` exists or that it should read it. So the skill text never reaches it, the index line is the whole skill, and the long-form entries cannot be slimmed for these agents.

## 4. Design

**R1. The index names the file.** For a provider other than Claude, an `agent-skill` line in the index gives its path, and the heading's sentence says when to read it:

```
# Available Skills

Read a skill's file before you act on what its line describes; the line is all you have until you do. …

- **github-gh-agent** — <description> (file: `.claude/skills/github-gh-agent/SKILL.md`)
```

The wording is the one the memory index uses (SPEC_MEMORY_DELIVERY_INDEX_NOT_FULL_TEXT_2026_11_17.md, section 4), so the two indexes read alike. A `prompt` skill keeps its `/<trigger>` line (some providers do have slash commands), with no file path. An `agent-skill` with no body writes no file, so its line names none.

**R2. The tool form.** The skill files are in the mounted workspace, so the relative path works in a container too. The heading's sentence also names the `Skills` tool (`list`, then `get`), which needs no file at all. Kimi has no startup file, so it gets no index from this change (D4).

**R3. Computer-use skills.** Their sibling files (references, approved scripts) are already written beside the `SKILL.md`; they are reached from the file the index names.

**R4. One path, no second copy.** The files stay at `.claude/skills/<slug>/SKILL.md`. A provider-neutral directory (`.agents/skills/`) would double every file, and the refresh, the cleanup manifest and the bundle export all know only `.claude/`. Which providers read `.claude/skills` natively is unverified and not needed (the only evidence in the code is Grok: `crates/srv-core/src/backend/providers/grok.rs` sets `GROK_CLAUDE_SKILLS_ENABLED=0`, so Grok can read them and AgentMux switches it off): the index names the file, so it works either way.

**R5. Size.** An index line is the description plus about 60 characters of path. With the 7 skills the catalog lists now (6 `agent-skill`, 1 `prompt`) that is about 1.5 KB at most, against the long-form text it replaces.

**R6. After R1 to R5 land, slimming the long-form Global Memory entries for non-Claude agents is a separate, later decision**, not part of this build: whether a non-Claude agent follows "read the skill first" as reliably as it follows the full text is what the check in section 6 measures.

## 5. Where it changes

- `crates/srv-core/src/backend/agent_config.rs`: `skills_index_text` (R1, R2). It is shared with `memory:launchcost` (`crates/srv/src/server/memory_launch_cost.rs`), whose estimate follows automatically.
- `crates/srv-core/src/backend/agent_config.rs` tests: new cases for an `agent-skill` with a file (the line names the path, and the path is a file `build_config_files` writes), a slash command (no path), an `agent-skill` with no body (no path), and Claude (its sentence unchanged).
- No change to the file writers, `managed_skill_file_paths`, the manifest or export.
- Changeset.

## 6. Check

The build ships on its unit tests alone (section 5): they pin the index text each provider gets, the path it names, and that the named file is one `build_config_files` writes. Nothing in the build waits on a model run.

A follow-up, optional and not part of this change: for two non-Claude providers (Codex and dsh), a headless run with the index of section 4 and a task a skill governs (a commit signature rule, an AWS denial), against the same task with the long-form text. It answers whether the long-form Global Memory entries can then be slimmed for those agents (R6). It is a model run, filed separately when it is wanted.

## 7. Not in scope

Per-provider native skill directories, installing skills outside the working directory, and changing the skill text itself.

## 8. Decisions

- **D1.** The index names the existing `.claude/skills` path; there is no `.agents/skills` copy.
- **D2.** No research into which providers read `.claude/skills` natively: a skill listed twice for one that does costs one line.
- **D3.** The section 6 run is a follow-up, not a condition of the build: the change merges on its unit tests.
- **D4.** Kimi (no startup file) gets no skills index from this build; reaching it through the memory fallback is a follow-up.
