# Spec: protect a pre-existing project `CLAUDE.md` from AgentMux's overwrite

**Date:** 2026-08-22
**Author:** Camper
**Status:** Implemented. The relative `@.claude/AGENTMUX_MEMORY.md` import
(the design's one open question) was smoke-tested directly against the
real Claude Code CLI (`claude -p`) before implementation — confirmed
working as assumed.
**Motivated by:** direct request, following the Armory "Global Memory" rename
(`SPEC_ARMORY_MEMORY_GLOBAL_PERSONAL_RENAME_2026_08_22.md`) — Global Memory
is supposed to represent "the global rules agents follow," but a project's
own real, git-tracked `CLAUDE.md` (e.g. this repo's own root `CLAUDE.md`)
is a second, very real source of exactly that, sitting entirely outside
AgentMux's system today.

## Problem — confirmed, and worse than a naming gap

`agent_config.rs::build_config_files()` composes `CLAUDE.md` from Soul +
AgentMD + Memory (Global Memory + per-agent) + a skills index, and both
call sites that materialize it to disk —
`agent_open.rs::write_agent_config_files()` (line 859) and
`editor_handlers.rs`'s `writeagentconfig` handler (line 336, "the actual
'click Launch' path") — write it with a raw `std::fs::write(&file_path,
&content)`. **No read of the existing file first, no diff, no backup, no
warning, no merge.**

The write target — `<agent.working_directory>/CLAUDE.md` — is not an
AgentMux-owned location. It's confirmed identical to Claude Code's own
native "project CLAUDE.md" discovery slot
(`docs/reports/REPORT_AGENT_AUTH_DIVERGENCE_2026_06_20.md`'s own location
table). **Any agent launched with its working directory pointed at a real
project that keeps hand-authored instructions in a root `CLAUDE.md` — this
repo included — has that file silently, fully replaced on every launch.**
Uncommitted edits at that moment are unrecoverable outside git.

The one nearby code comment (`agent_open.rs:582-587`, "No collision
resolution... overwrites whatever's there") is about concurrent AgentMux
launches racing on the same *synthetic* workdir, not about a pre-existing
independently-authored file — nobody appears to have reasoned about this
case for Claude specifically.

**A correct precedent already exists in this codebase, for a different
provider.** `SPEC_CODEX_PROVIDER_INTEGRATION_2026_08_08.md` §10.2: *"AgentMux
must not overwrite, merge by string marker into, or claim ownership of an
existing repository `AGENTS.md`"* — routing AgentMux's content through an
AgentMux-owned side file (`$CODEX_HOME/agentmux-<id>.config.toml` +
`--profile`) instead. That design is itself unimplemented ("Slice E... not
started"), and — the actual gap this spec closes — **was never extended to
Claude/`CLAUDE.md`, which is the path that's live and actively overwriting
real files today.**

## Constraints ruled out by research (read before proposing alternatives)

- **`CLAUDE_CONFIG_DIR`-relocated user-level `CLAUDE.md` is not a safe side
  channel.** It's resolved per identity/account binding, not per agent —
  multiple unbound agents share one instance-wide directory, and an agent
  bound to the legacy "Default" migration identity resolves it to the
  user's real global `~/.claude`. Writing AgentMux content there risks
  leaking between agents or clobbering the user's own real global config
  (`identities.rs:80-86`'s own doc comment confirms the Default-bundle case
  literally resolves to `~/.claude/`).
- **No `--system-prompt`/`--append-system-prompt` flag is used anywhere in
  this codebase today.** It's a real Claude Code CLI flag
  (`docs/research/claude-code-presentation-layer.md` confirms it exists),
  but introducing it here would be new integration work, not reuse of an
  existing path — and its compatibility with AgentMux's actual persistent
  launch mode (`--input-format stream-json --output-format stream-json
  --permission-prompt-tool stdio`, `providers.rs`'s `persistent_launch_args`)
  is unverified. Soul/AgentMD/Memory are all funneled into the same
  `CLAUDE.md` today — there's no existing split to extend.
- **No project-scoped tier exists in the data model** — `db_bundles` has
  only a boolean `is_global`, no working-directory/repo column. This spec
  doesn't add one; it's a file-write-safety fix, not a new memory tier.

Given these, a fully zero-touch fix isn't achievable today without taking
on unverified `--append-system-prompt` integration risk. §Design below is
a minimal-touch fix deployable now; §Future work names the zero-touch path
as a follow-up once that flag's compatibility is verified.

## Design

### Ownership tracking — reuses the existing managed-files philosophy

`agent_config.rs` already has exactly this pattern for skill files:
`MANAGED_SKILL_FILES_MANIFEST` (`.claude/.agentmux-managed-skill-files.json`)
tracks which paths AgentMux itself created, so `cleanup_stale_managed_skill_files`
never deletes a file the user hand-authored outside that manifest. This
spec applies the same *distinguish AgentMux-authored from user-authored*
principle to `CLAUDE.md`, via a lighter mechanism suited to tracking a
single file rather than a dynamic set:

1. **A one-line marker as the first line of any `CLAUDE.md` AgentMux
   itself writes:** `<!-- agentmux:managed-claude-md -->`. On every write,
   check the *existing* file first:
   - **Doesn't exist:** create it as today (full Soul+AgentMD+Memory+Skills
     composition), with the marker as line 1. No behavior change from
     today for the common case — a fresh working directory.
   - **Exists and starts with the marker:** AgentMux wrote it last time.
     Safe to regenerate in place exactly as today.
   - **Exists and does NOT start with the marker:** foreign — either
     predates AgentMux touching this workdir, or a human replaced
     AgentMux's file with their own. **Never overwrite its content.**
2. **Foreign-file case: deliver AgentMux's content via a side file +
   a single idempotent `@import` line**, not a full-file replacement:
   - Write the full Soul+AgentMD+Memory+Skills composition to an
     AgentMux-owned file under the same namespace the skill manifest
     already uses — `.claude/AGENTMUX_MEMORY.md` — freely regenerated on
     every launch (it's 100% AgentMux's own content, always safe to
     rewrite).
   - Append exactly one line to the **end** of the real `CLAUDE.md`,
     wrapped in a comment so its origin is unambiguous:
     ```
     <!-- agentmux:managed-import (safe to delete this line to opt out) -->
     @.claude/AGENTMUX_MEMORY.md
     ```
     via Claude Code's own `@path` import mechanism
     (`docs/reports/REPORT_AGENT_AUTH_DIVERGENCE_2026_06_20.md:261-262`
     confirms `@path` imports are real and already used for the analogous
     user-level case). **This never touches the user's own content above
     it** — append-only, and the insertion is idempotent (checked before
     inserting, never duplicated on repeat launches).
   - **Respecting an explicit opt-out:** whether the import line has
     already been offered for this working directory is tracked in a
     small on-disk marker, `.claude/.agentmux-claude-md-ownership.json`
     (`{ "import_line_offered": true }`), checked *instead of* re-scanning
     file content on every launch. Without this, a user who deliberately
     deletes the import line (opting out of AgentMux content entirely)
     would see it silently reappear on their next launch — the marker file
     is what lets AgentMux tell "never offered yet" apart from "offered
     and removed on purpose," and only ever inserts the line once per
     working directory.

### Open questions / needs verification before implementation

- **Relative `@path` resolution.** The confirmed example
  (`@~/.claude/my-instructions.md`) is home-relative; this design needs a
  plain-relative `@.claude/AGENTMUX_MEMORY.md` resolved against the
  including file's own directory. Needs a real Claude Code smoke test
  before shipping — if relative imports don't resolve the way this
  assumes, the side file may need an absolute path instead (computable at
  write time either way, no design change, just confirm which form Claude
  actually expects).
- **Does the LLM "seeing" the marker comment matter?** An HTML comment is
  invisible to a Markdown renderer but not to the model reading the raw
  file — it'll see `<!-- agentmux:managed-claude-md -->` as ordinary text.
  Harmless (one line of context), just noting it's not truly hidden from
  the agent the way it would be from a human viewing rendered Markdown.

## Future work — the zero-touch path

If `--append-system-prompt` is verified compatible with AgentMux's
persistent stream-json launch mode, a follow-up spec could deliver
Soul/AgentMD/Memory via that flag instead of any file at all for agents
whose `CLAUDE.md` is foreign — fully eliminating even the one-line
`@import` touch. That's real, separate integration work (flag plumbing
through `providers.rs`'s launch-args, verifying it doesn't interact badly
with `--permission-prompt-tool stdio`) — not bundled here, since this
spec's fix (stop the active data-loss risk) is deployable now without it.

## Non-goals

- Not building a project-scoped memory tier in the data model — Global
  Memory and per-agent bundles stay exactly as they are; this only fixes
  *how* their composed content reaches disk when a foreign `CLAUDE.md`
  is present.
- Not implemented for Codex/`AGENTS.md` here — that's
  `SPEC_CODEX_PROVIDER_INTEGRATION_2026_08_08.md` §10.2's job, already
  spec'd, separately unimplemented, and out of scope for this Claude-specific
  fix.
- Not retroactively repairing any `CLAUDE.md` that's already been
  overwritten by AgentMux before this ships — no way to distinguish
  "AgentMux clobbered real content" from "AgentMux legitimately created
  this from nothing" after the fact without the marker this spec
  introduces going forward. Recovery for anyone already affected is git
  history/reflog, same as today.

## Testing

- Unit: `agent_config.rs`'s ownership-detection logic — marker present vs.
  absent vs. no file at all, each producing the right write behavior.
- Unit: idempotent import-line insertion — inserting twice must not
  duplicate the line; a user-deleted import line must not reappear
  (mocked ownership-marker file state).
- Integration/manual: launch an agent against a real directory with a
  hand-authored `CLAUDE.md` (this repo is the natural test case in a
  throwaway clone — never test against a real checkout in place) and
  confirm its content is untouched, `.claude/AGENTMUX_MEMORY.md` contains
  the composed Soul+AgentMD+Memory+Skills content, and Claude Code
  actually resolves the `@import` (the open question above) — verify with
  a prompt that only the imported content could answer correctly.
