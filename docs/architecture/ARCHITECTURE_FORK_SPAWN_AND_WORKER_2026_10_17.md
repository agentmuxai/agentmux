# Fork, spawn and worker

**Date:** 2026-10-17
**Status:** living
**Why this doc:** two features make a new agent from an existing one, and the
words for them were used loosely. This is the one definition both features'
code, comments and specs point to. The in-code pointer is the "Fork, spawn and
worker" comment at the top of `crates/srv/src/server/agent_handlers/mod.rs`.

## Fork

The user's: the pane's **Quick Fork** (`frontend/app/view/agent/quick-fork.ts`)
and the picker's **Fork** (`AgentPicker.tsx`, `handleFork`).

A fork is an **exact copy** of its agent, conversation included:

- **Definition:** copied by `forkagentdefinition` (`template::fork_definition`),
  with a fresh id, workspace and bundle, linked to its parent by `parent_id`
  and to the parent's account.
- **Conversation:** the CLI continues the parent's conversation with
  `--fork-session`. It reads the parent's session and writes its own, so it
  never writes into the parent's.
- **Pane:** opens with the parent's history. `copyforkhistory`
  (`agent_session::fork_history`) copies the parent's transcript into the
  fork's before it launches, when the fork continues the parent's last
  session on a provider that can (Claude today). A fork on any other provider
  starts fresh, with an empty pane and a note saying so.
- **Safety:** the fork's copied history ends in the parent's session. A
  process spawn that resumes that session from the fork always forks it
  (`persistent/spawn.rs`), however the session id got there.

## Spawn, and the worker it makes

An agent's: the `Agents` tool's **spawn** action (`app_api/agent_spawn.rs`,
`SPEC_AGENT_WORKERS_2026_10_15.md`).

It makes a **worker**, a new agent:

- built from the leader's definition (soul, instructions, skills, via
  `copy_definition`);
- with the leader's **brief** injected as its first message, and Global Memory
  like any launch;
- with **no conversation history** and its own empty Personal Memory.

A worker is not one of the leader's forks, and spawning never copies history.

## Process spawn

Starting the provider CLI for a pane (`blockcontroller/persistent/spawn.rs`,
"first spawn", `spawn_forks`). Every agent's process starts this way, fork,
worker or neither; it is not the `Agents` action. In new code and comments,
say "process spawn" or "launch" where "spawn" alone could mean either.

## A fork of a worker

A fork of a worker is an independent agent (operator's decision, 2026-10-17).
It carries the worker's conversation, brief included, like any fork, but it
is not the leader's worker: it doesn't count toward the leader's limit and it
may spawn workers of its own. `copy_definition` leaves out the worker mark
(`agent:worker_of`) when it builds a fork.
