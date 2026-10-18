# SPEC: each agent's Bash calls keep their own working directory

**Date:** 2026-10-18
**Status:** active — built in #5096.
**Author:** Naki #2 (naki-naki-2), at the repo owner's request.
**Builds on:** the cwd persistence in `crates/bashwrap/src/cwd_state.rs` (`CwdState`, moved there from `bash_wrap.rs` by this change; `docs/retro/RETRO_BASH_CWD_RESET_NOTICE_WINDOWS_2026_08_02.md`).

## 1. Problem

bashwrap keeps an agent's working directory between its one-shot Bash calls. After each call it writes the shell's ending `$PWD` to a state file, and the next call starts there. The file is keyed per AgentMux instance (`AGENTMUX_INSTANCE_SLUG`).

A Claude Code session that starts subagents (the Agent tool) runs all of their Bash calls under that same instance, with the same environment, so they all shared one file. When a subagent ran `cd ../its-own-clone`, the main agent's next command started in the subagent's clone.

Seen on 2026-10-17, with four helpers each working in its own clone:
- three times a `git add -A && git commit` committed a helper's unfinished work under the main agent's message, in the wrong clone;
- tests and reads ran against the wrong clone.

Nothing failed, so it was found only by reading branch names in output.

## 2. Change

- **Hook:** Claude Code's PreToolUse hook input carries `agent_id` when a subagent made the call; the main conversation's calls have none. The hook (`hook.rs`) reads it and passes it on in both of bashwrap's modes:
  - rewrite mode: `agentmux-bashwrap exec … --agent=<id>`;
  - prefix mode: an `agent_id` field on the call record (`prefix.rs`).
- **`CwdState::load(agent)`:**
  - The main conversation keeps the instance's file, `<instance>.cwd`, unchanged. srv's reader of it (`work_facts/paths.rs`) is unaffected, and it no longer sees helpers' directories.
  - A subagent uses `<instance>.agent-<agent_id>.cwd`. It starts from its own file, or on its first call from the instance's, so a new helper starts where the main agent is. It writes only its own file.
- **Sweep:** the main conversation's calls delete subagent files untouched for 7 days. Subagent ids are per spawn, so their files would otherwise pile up. An instance's own file is never swept.

## 3. Not changed

- What Claude Code itself believes the working directory is (its "Shell cwd was reset" notices). That is the CLI's, out of process.
- Two parallel calls of the same agent still race on its one file, as before (`append_cwd_capture`'s note).

## 4. Tests

`cwd_state.rs`:
- `a_subagents_cd_never_moves_the_main_agent`: a new subagent starts where the main agent is; its `cd` moves only itself; another subagent and the main agent are unaffected; an empty id is the main agent.
- `subagent_state_files_are_swept_but_an_instances_own_never_is`.

`hook.rs`:
- `a_subagents_call_names_the_agent_and_the_main_conversation_none`;
- `record_mode_keeps_the_subagent_on_the_record`.
