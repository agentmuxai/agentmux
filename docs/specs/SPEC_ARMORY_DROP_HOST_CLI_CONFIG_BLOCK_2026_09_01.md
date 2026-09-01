# Spec: Drop the "Claude Code — host CLI config" block from Armory Global Memory

**Date:** 2026-09-01
**Status:** Proposed
**Motivated by:** direct request — *"now that we know leaks are squashed, we
don't need the host cli config at all in the armory."*

## Problem

Armory → Memory → Global renders an "External Claude Code files" section with
two read-only reference blocks
(`frontend/app/view/brain/global-brain-manager.tsx`):

| Block | Path shown | Caption | Keep? |
|---|---|---|---|
| Claude Code — **shared provider config** | `DataPaths::provider_auth_dir("claude")` (`~/.agentmux/shared/providers/claude/CLAUDE.md`) | "Used by default spawned agents." | **Yes** |
| Claude Code — **host CLI config** | `~/.claude/CLAUDE.md` | "Used outside AgentMux." | **No — remove** |

The host block was added by
`SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md` §6–§7 at a time when it was
genuinely unclear which file a spawned agent read. That ambiguity is now
resolved: `REPORT_CLAUDE_CONFIG_DIR_ISOLATION_EVIDENCE_2026_09_01.md` proved
by controlled experiment that a spawned agent reads
`$CLAUDE_CONFIG_DIR/CLAUDE.md` and — once seeded, which
`prepare_provider_auth_dir()` now guarantees at every spawn — **never** the
host file.

So the host block now shows a file that has no bearing on any AgentMux agent.
Its own tooltip already says as much ("Not read by spawned in-app agents").
Surfacing a file in Armory whose entire description is "this does not affect
anything here" is noise, and worse, it invites the misreading that editing it
would change agent behaviour.

## Design

Remove the host-CLI-config block and its entire supporting chain. Keep the
shared-provider block: that one shows the file agents genuinely do read, and
is now *more* useful than before — post-fix it displays the AgentMux isolation
placeholder, which is a direct, legible confirmation that isolation is in
effect.

### Frontend

- `frontend/app/view/brain/global-brain-manager.tsx` — delete the second
  `<Show when={model.claudeHostConfigAtom()}>` block; narrow the wrapping
  `<Show>` condition to `model.claudeGlobalConfigAtom()` alone.
- **Reword the section heading.** "External Claude Code files" only made sense
  as a plural covering both. The single remaining block is not "external" at
  all — it is AgentMux's own isolated provider dir. New heading: *"Claude Code
  provider config — reference only, not part of Global Memory."* The
  "reference only, not part of Global Memory" clause is load-bearing and stays
  (it is what stops the block being mistaken for an editable Global Memory
  section) — only the misleading "External … files" framing goes.
- `frontend/app/view/brain/global-brain-model.ts` — remove the
  `_claudeHostConfig` signal, `claudeHostConfigAtom`, `setClaudeHostConfig`,
  and the `GetClaudeHostConfigCommand` fetch.
- `frontend/app/store/rpc-api/memory.ts` — remove `GetClaudeHostConfigCommand`.
- `frontend/app/view/brain/global-brain-model.test.ts` — remove the three
  `claudeHostConfigAtom` tests.

### Backend

- `agentmux-srv/src/server/agent_handlers/memory.rs` — remove the
  `COMMAND_GET_CLAUDE_HOST_CONFIG` handler.
- `agentmux-srv/src/backend/rpc_types/commands.rs` — remove the
  `COMMAND_GET_CLAUDE_HOST_CONFIG` constant.
- **Keep** `read_claude_global_config()` and
  `resolve_shared_claude_provider_dir()` — both still serve the surviving
  `getclaudeglobalconfig` handler. Keep that function's tests; update the
  module comment that currently says it "covers both" handlers, since it will
  cover one.

## Non-goals

- **No change to the shared-provider block.** It is the useful half and gets
  more useful post-isolation-fix.
- **No change to isolation behaviour.** This is a pure UI/RPC-surface removal;
  nothing here touches `prepare_provider_auth_dir()`, the seeding, or any
  spawn path. Removing the *display* of the host file cannot affect whether it
  is *read* — that is settled independently and proven.
- **No removal of `SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md`.** It stays
  as the record of why the block existed; this spec supersedes only its §6/§7
  host-block half.
