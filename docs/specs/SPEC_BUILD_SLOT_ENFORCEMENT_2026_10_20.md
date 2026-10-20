# Spec: The build slot — one heavy build or test at a time on a machine, enforced for agents' cargo

**Status:** active. Shipped: §3 (the lock script) and §4 (the `cargo` shim, installed at startup) in this PR. Left: the same for the other heavy commands (§6), the holder on the Swarm and Presence views (§6), and wait/hold reporting (§6).
**Date:** 2026-10-20
**Author:** Lazo
**Verified against:** `agentmux` `main` @ `2a864efd4` (2026-10-20), by reading code.

## 1. Ask

The operator, 2026-10-20, after two agents compiled the same crate at once and the machine ran out of CPU and memory: "we need a new protocol to where agents only do one build or test at a time." Then: "proceed with both", meaning the written standard and enforcement in AgentMux, so agents cannot forget the rule.

## 2. What happened, and why a rule is not enough

Two agents ran `cargo test -p agentmux-srv` in separate clones within a minute of each other. Four `rustc` processes held about 19 GB of memory and the CPU sat at 100% on 8 cores; the operator stopped both runs by hand. Neither agent did anything wrong by the rules that existed: nothing told them to take turns. Running both at once finishes neither sooner than running them one after the other, and it slows everything else on the machine.

A written rule fixes that for the agents that read it at their next launch. It does not reach an agent that is already running, one that skims, or a script that calls cargo. So the lock is a tool, and the tool is put in the way: an agent's `cargo` already goes through it.

## 3. The lock

`scripts/build-slot/build-slot.sh`, installed to `~/.agentmux/shared/locks/build-slot.sh`:

```
build-slot.sh [--reason "text"] -- <command> [args...]     run <command> when the slot is free
build-slot.sh status                                       who holds the slot, who waits
```

- **One slot per machine**, shared by every agent, clone, repository and AgentMux channel: the directory `~/.agentmux/shared/locks/build/slot`, created with `mkdir` (atomic).
- **A queue**: a waiter takes a ticket and runs in order of arrival. It prints who holds the slot and for how long, and gives up after 60 minutes with exit code 75.
- **A dead holder frees the slot.** The holder refreshes a heartbeat file every 20 s; a slot with no heartbeat for 90 s is taken over. Each holder writes a unique token and removes the slot only if the token is still its own, so a late exit cannot free the slot under its successor.
- **Nesting**: a command started inside the slot (`task package` calling cargo) runs without waiting for itself (`BUILD_SLOT_HELD=1`).
- **The wrapped command's exit code** is the script's exit code.
- **Needs only bash**, `mkdir`, `date`, `stat` and `sleep`: Git Bash on Windows, Linux and macOS.

The standard that tells agents what counts as heavy and how to behave is `docs/standards/DEV_HOST_BUILD_SLOT.md` in the private infrastructure repository; this spec covers what AgentMux does about it.

## 4. The enforcement: a `cargo` in the way

Agent shells already put `~/.agentmux/tools/bin` ahead of the system PATH (`tool_store::tools_path`, `UserToolsPrecedence::BeforeExisting`, used for every agent turn in `server/agent_handlers/input.rs`). At startup srv installs two files there (`agentmux_common::build_slot`, called from `bootstrap/background.rs`):

- `cargo`, a bash script, for Git Bash and Linux and macOS shells;
- `cargo.cmd`, for `cmd.exe` and PowerShell, which hands over to `cargo` through Git for Windows' bash.

The shim finds the real cargo (the next one on PATH that is not its own directory), and for the heavy subcommands runs it as `build-slot.sh -- <real cargo> <args>`:

`build` `check` `test` `clippy` `doc` `bench` `rustc` `rustdoc` `fix` `install` `package` (and their short forms), and the common third-party `nextest` `llvm-cov` `tarpaulin` `udeps` `miri`. A `+toolchain` or option before the subcommand is skipped when finding it. Everything else (`metadata`, `fmt`, `tree`, `update`, `clean`, …) runs cargo directly.

**`cargo run` is deliberately not slotted.** It builds, but then keeps running a server or the app, and the slot must not be held for as long as that.

**It fails open.** No lock script, no bash, no real cargo on the PATH beyond the shim's own directory (that case exits 127 with a message): in every other failure cargo runs as if the shim were not there. `AGENTMUX_BUILD_SLOT_OFF=1` skips the slot for one command.

**Installing is safe to repeat and to share between channels:**
- The script is replaced only by a newer `# build-slot-version: N`, so an older AgentMux on the same machine never overwrites a newer one's.
- A shim is replaced only when it carries the shim's marker. A `cargo` somebody put there is left alone (and logged).
- Files are written to a temporary name and renamed into place, so a shell starting a shim sees the old file or the new one, never half of one.

**Who this reaches:** agent turns (the PATH above). Interactive terminal panes put the user's tools dir after the system PATH (`UserToolsPrecedence::AfterExisting`), so a person's own `cargo` there is never slotted. A tool that calls `cargo.exe` by full path bypasses it, as does an agent that sets `AGENTMUX_BUILD_SLOT_OFF`; the standard says not to.

## 5. Tests

- `agentmux_common::build_slot`: the installer writes once and then does nothing; leaves a foreign `cargo` in place; replaces an older shim; never downgrades the script; replaces an unversioned script; the embedded files carry the version line and marker and have the right line endings.
- The shim itself, run under bash with a stand-in cargo and a stand-in lock script: a heavy subcommand (including after `+nightly` and an option) goes through the slot; `metadata`, `run` and no subcommand do not; no lock script and `AGENTMUX_BUILD_SLOT_OFF` both run cargo directly. Skipped where there is no bash.
- The lock script was exercised by hand for contention and queue order, exit-code passthrough, a nested call, takeover of a dead holder, a captured-output caller, and a late exit that must not free a successor's slot.

## 6. Next

1. **Other heavy commands**: `task package`, `vite build`, the full vitest and node suites and Docker builds go through the slot only when the agent runs them by hand. The same shim mechanism can cover `docker` and a `vite` front, but a `task` or `npm` shim would slot light invocations too, so those want a subcommand list of their own.
2. **Show the holder**: put it in `Presence` and the Swarm view, so a person sees "AgentX holds the build slot, 12 min" without running `status`.
3. **Report waits and holds**, to tune the heavy list.
4. **A counted slot**: a machine with more cores than agents' builds need could allow two. One is the safe start.
5. **Non-Claude providers**: this covers the agent turns that use the shared environment builder. Check each other provider's spawn (ACP and terminal tiers) puts the user tools dir first.
