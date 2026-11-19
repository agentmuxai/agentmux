# Spec: a container agent runs as one long-lived process, like a native agent

**Date:** 2026-11-19
**Status:** draft. Awaiting review; the build is not started.
**Task:** #47962 part B.

## 1. Problem

A native agent is one long-lived process fed over stdin. Its turn ends at the CLI's `result` event, the
process stays up, and a message sent at any time reaches it within moments.

A container agent runs one `docker exec` per turn (`container_spawn.rs`, `container_spawn_steps.rs`). The CLI
reads its prompt from a file, its stdin is at end of file, and the turn ends when the exec exits. When the model
sends a command to the background, the CLI answers (the `result` event is out) but stays alive until that
command finishes, because it has to deliver the command's completion to the model. The exec therefore does not
exit, the run lock stays held, and every message sent meanwhile is queued behind it
(`queue_behind_running_process`) and drained one per exec exit. A person's message can wait for as long as the
background command runs (a build, a test run, a watch): tens of minutes, with nothing the agent or the person
can do except stop the agent.

A native agent does not have this: the message goes to stdin and the CLI takes it between the background
command's steps.

## 2. Why not simply a second exec on the same session

Tried on the real CLI (v2.1.301): process 1 starts a background `sleep` and reaches its result event, still alive;
process 2 resumes the same session id and is asked to remember a word and answers; process 1's command finishes
and process 1 writes the completion turn; process 3 resumes the session and is asked for the word.

Result: process 3 does not know the word. The session file is a tree (each entry has `uuid` and `parentUuid`),
each live process appends from its own in-memory parent, so the two exchanges fork the session at process 1's
answer and the later leaf hides the other branch. The person's message was answered but the history forgot it.
Two writers on one session are not safe; the fix has to keep one process per session.

## 3. Requirements

R1. A message sent to a container agent is delivered to its CLI within seconds when the CLI is between steps,
including while a background command runs, without a second process on the session.

R2. The turn ends at the `result` event, not at the process's exit. The pane's working state, the turn ledger,
the update gate and the queue drain follow the result event, as they do for a native agent.

R3. The process survives between turns. When it ends (the container restarts, srv restarts, the CLI exits), the
next message starts a new one with `--resume <session id>`, as a native agent does after a srv restart.

R4. A stop interrupts the turn (an interrupt control request) and ends the process only when asked to, or after
the interrupt has not been honoured within a bounded time. It never kills another block's CLI in a shared
container.

R5. Nothing changes for a native agent. A container agent that cannot run the persistent path (an image without
the tools, a provider that takes its message on the command line, Codex) keeps the per-turn exec.

R6. While R1 is not in place for a provider, a person's message never waits silently behind background-only
work: the pane says what it waits for and offers "Stop it and send now" (interrupt, then send) and keeps
"send when it finishes" as the default.

## 4. Design

The `PersistentSubprocessController` is tied to `tokio::process::Child` in about six places: starting the child
and taking its pipes (`spawn_steps`: `start_child`, `take_pipes`), the stdin writer, the process waiter, the
stderr reader and the pid bookkeeping. The transport itself is not the obstacle: a live exec's stdin is a plain
duplex stream, and the "no end of file" limit noted in `container.rs` only hurts CLIs that read until EOF, which
the stream-json input mode does not.

1. Introduce a small process trait that the persistent controller uses instead of `Child`: write stdin,
   read stdout and stderr as streams, wait for exit (with exit code), interrupt, kill, pid. A host
   implementation wraps `tokio::process::Child`; a container implementation wraps a bollard exec started with
   attach and stdin open.
2. The container implementation starts the CLI as `claude -p --input-format stream-json --output-format
   stream-json ...` (the same flags a native agent gets) under a wrapper that writes its own pid to a file in the
   container, so a kill targets that process only.
3. Exit: read the exec's exit code by inspecting the exec once the output stream ends, and also poll the
   inspection while the stream is open (today `read_output` ends only on stream end and never polls).
4. Drop: when srv's side of the exec stream drops, the CLI sees stdin end and exits at once; a second attach to the
   same exec is refused ("already running"); a container restart ends it (exit 137). So there is no re-attach:
   on any of these, R3 applies.
5. Stop: an interrupt control request on stdin (as for a native agent), then, after a bounded wait, a kill by the
   recorded pid. There is no EOF-based graceful stop.
6. The prompt-file path (`container_turn_exec`) and the per-turn queue (`pending_messages`, drained per exit)
   remain for the non-persistent fallback (R5).

## 5. Open points to settle in the build

- Session files: the resume gate and relocation read the session file on the host; confirm they can read the
  file a long-lived container process writes, or make them ask the container.
- Token lifetime: the per-turn credential route (`key_route`) is bound to a spawn; a process that lives for days
  needs a token that outlives a turn, or a refresh path.
- Update gate: the container update gate is held for the whole turn today and would block updates forever if a
  turn were the process's life; it has to be held per result-bounded turn.
- The compact hint, `Agents fresh`, the turn ledger and the queue drain all assume "turn = exec": each moves to
  "turn = result event".
- An image that predates the tools (R5) and the resume path after a container restart.

## 6. Phases

1. Process trait with the host implementation only; the persistent controller's tests unchanged and green.
2. Container implementation behind a setting, default off; the fake Docker daemon in the srv-core tests grows an
   attach-with-stdin mode.
3. Turn end at the result event for container agents; update gate and ledger follow it; drain per result.
4. Default on; remove the per-turn path for providers that support it (keep it as the R5 fallback).

## 7. Tests

- A message sent while a background command runs is delivered to the running process's stdin, and no second
  exec is started for the session.
- The background command's completion reaches the agent exactly once, and the answer to the message sent meanwhile
  is in the session when the session is resumed.
- Two messages sent while the agent is busy are delivered in the order sent, each once.
- The process ending (container restart, srv restart, CLI exit) starts a new one with `--resume`, losing nothing.
- A stop interrupts the turn and leaves the process up; a forced stop kills only this block's process.
- The update gate is free between turns while the process stays up.

## 8. Effort

Phases 1 and 2 are the bulk: a few days with the fake daemon work. Phase 3 and the open points in section 5
need the container's session files and the token lifetime settled first.
