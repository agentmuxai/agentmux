# Spec: The build slot — one heavy build or test at a time on a machine, enforced for agents' cargo

**Status:** active. Shipped: §3 (the lock script) and §4 (the shims for `cargo`, `task`, `npm`, `npx` and `docker`, installed at startup; #5207, #5379, #5386), the wait/hold history (§6 item 3, #5391) and the holder in `Presence who` (#5406) and the Swarm row (§6 item 2). Left: a check of the shims on a fresh install of a release that carries them (§6), and a counted slot only if the history shows queues (§6 item 4).
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
- **A wait whose parent is gone leaves the queue** (a stopped background task, a closed pane), and a signal stops a waiter within a poll. This only applies when the script can watch its parent: Git Bash reports pid 1, and cannot signal it, when the parent is a native Windows program (`cmd.exe`, PowerShell, `task.exe`), which is how the `cargo` shim usually reaches the script. In that case the check is off, because taking pid 1 for a dead parent made every wait started that way leave at once with exit 129 (found 2026-10-21, fixed in version 5). `BUILD_SLOT_DETACHED=1` turns the parent check off for a wait started on purpose with `nohup` or `setsid`.
- **The whole script is one function**, called on its last line, and it is installed by writing a temporary file and renaming it. Bash reads a script a piece at a time, so a file rewritten in place under a running wait broke that wait mid-line (it happened on 2026-10-20, to three agents' queued runs).
- **Nesting**: a command started inside the slot (`task package` calling cargo) runs without waiting for itself (`BUILD_SLOT_HELD=1`).
- **The wrapped command's exit code** is the script's exit code.
- **Needs only bash**, `mkdir`, `date`, `stat` and `sleep`: Git Bash on Windows, Linux and macOS.

What counts as heavy and how agents behave around the slot (queue etiquette, not killing another agent's build) is the team's dev-host standard and is given to agents in their instructions; this spec covers what AgentMux does about it.

## 4. The enforcement: a shim in the way

Agent shells already put `~/.agentmux/tools/bin` ahead of the system PATH (`tool_store::tools_path`, `UserToolsPrecedence::BeforeExisting`, used for every agent turn in `server/agent_handlers/input.rs`). At startup srv installs a shim there for each heavy tool (`agentmux_common::build_slot`, called from `bootstrap/background.rs`): `cargo`, `task`, `npm`, `npx` and `docker`. Each is the same two files under the tool's name:

- `<tool>`, a bash script (`scripts/build-slot/shim`), for Git Bash and Linux and macOS shells;
- `<tool>.cmd` (`scripts/build-slot/shim.cmd`), for `cmd.exe` and PowerShell, which hands over to the sibling bash script through Git for Windows' bash.

The shim acts for the tool whose name it carries. It finds the real tool (the next one on PATH that is not its own directory; a `.cmd` is the last resort), and for the heavy subcommands runs it as `build-slot.sh -- <real tool> <args>`. A tool's light subcommands run it directly. What is heavy, per tool:

- **`task`**: `package`, `build` and `bundle` (and their `:`-suffixed variants such as `package:portable` and `build:backend`), and `storybook:build`. `dev` and `run` are not slotted, for the reason `cargo run` is not (the app keeps running); the cargo calls inside `task dev` are slotted by the `cargo` shim anyway. Several names may be given: one heavy name slots the call.
- **`npm`**: `npm run` for `build`, `build:*`, `package`, `package:*`, `coverage` and `test:e2e`, and `npm run test` / `npm test`. The last two are the bare `vitest`, which watches with a terminal and runs once without one, so they are slotted when stdin is not a terminal (an agent's shell has none), or when the arguments ask for a single run (`npm test -- --run`). `--coverage` alone leaves vitest watching, so it does not count.
- **`npx`**: `vite build`, `vitest run` (or `--run`), and a bare `vitest` without a terminal. `npx tsc` and the other quick tools are not slotted.
- **`docker`**: `build`, `buildx build`, `builder build`, `image build` and `compose build` or `compose up --build`. `run`, `ps` and plain `compose up` are not.

For `cargo`, the heavy subcommands are:

`build` `check` `test` `clippy` `doc` `bench` `rustc` `rustdoc` `fix` `install` `package` (and their short forms), and the common third-party `nextest` `llvm-cov` `tarpaulin` `udeps` `miri`. A `+toolchain` or option before the subcommand, with the separate value of `-C`, `-Z`, `--config` and `--color`, is skipped when finding it. Everything else (`metadata`, `fmt`, `tree`, `update`, `clean`, …) runs cargo directly.

**`cargo run` is deliberately not slotted.** It builds, but then keeps running a server or the app, and the slot must not be held for as long as that.

**It fails open.** No lock script, no bash, no real cargo on the PATH beyond the shim's own directory (that case exits 127 with a message): in every other failure cargo runs as if the shim were not there. `AGENTMUX_BUILD_SLOT_OFF=1` skips the slot for one command.

**Installing is safe to repeat and to share between channels:**
- The script is replaced only by a newer `# build-slot-version: N`, so an older AgentMux on the same machine never overwrites a newer one's.
- A shim is replaced only when it carries the shim's marker. A `cargo` somebody put there is left alone (and logged).
- An isolated instance (`AGENTMUX_HOME_OVERRIDE` set: a test or capture run) installs nothing. The lock and the shims are shared by every AgentMux on the machine, and `install_for_current_user` writes to the OS home, not the override root, so an isolated start would otherwise change what every agent runs (found 2026-10-24 while preparing the v0.59.41 checks).
- Files are written to a temporary name and renamed into place, so a shell starting a shim sees the old file or the new one, never half of one.

**Who this reaches:** agent turns (the PATH above). Interactive terminal panes put the user's tools dir after the system PATH (`UserToolsPrecedence::AfterExisting`), so a person's own `cargo` there is never slotted. A tool that calls `cargo.exe` by full path bypasses it, as does an agent that sets `AGENTMUX_BUILD_SLOT_OFF`; the standard says not to.

## 5. Tests

- `agentmux_common::build_slot`: the installer writes once and then does nothing; leaves a foreign `cargo` in place; replaces an older shim; never downgrades the script; replaces an unversioned script; the embedded files carry the version line and marker and have the right line endings.
- The shim itself, run under bash with a stand-in cargo and a stand-in lock script: a heavy subcommand (including after `+nightly` and an option) goes through the slot; `metadata`, `run` and no subcommand do not; no lock script and `AGENTMUX_BUILD_SLOT_OFF` both run cargo directly. Skipped where there is no bash.
- The lock script was exercised by hand for contention and queue order, exit-code passthrough, a nested call, takeover of a dead holder, a captured-output caller, and a late exit that must not free a successor's slot.

## 6. Next

1. ~~**Other heavy commands**~~ (done): `task`, `npm`, `npx` and `docker` have shims with a heavy-subcommand list each (§4). A direct `vite` or `vitest` binary is not shimmed: it is in `node_modules/.bin`, not on the tools PATH, so `npm run` and `npx` are the ways in.
2. **Show the holder**: in `Presence who` (done): `agentmux_common::build_slot::holder()` reads the lock's owner file (a slot with no heartbeat for 90 s is a dead holder and shows nothing), and the entry of the agent holding it carries `build_slot: {since_ms, reason, command}`. The lock names the holder by `AGENTMUX_AGENT_ID` (the slug), so srv matches the slug or the name; a leader's background subagent holds the slot under the leader's id, which is why the entry also carries the command line (cut to 120 characters). Local agents only. In the Swarm (done): the typed RPC `getbuildslot` returns the holder and the block of the local agent that matches it (or null), and the holder's row gets a muted line, `Holding the build slot · 12 min · cargo test`, with the whole command in its tooltip (`BuildSlotNote` in `swarm-row-notes.tsx`; the poll is `swarm-build-slot.ts`, every 15 s, one for any number of rows, only while a row is on screen). Nothing for LAN or mobile: the fleet feed is unchanged.
3. ~~**Report waits and holds**~~ (done in script version 6): each run, and each wait that gave up, appends a line (epoch, agent, reason, seconds waited, seconds held, exit code) to `~/.agentmux/shared/locks/build/history.log`, kept in two generations (past 2000 lines it is renamed to `history.log.1`, so nothing is rewritten and no concurrent append is lost); a run cut short by a signal is recorded with exit 130. `build-slot.sh history` summarises it by reason. This is the data for deciding whether a counted slot (item 4) would help.
4. **A counted slot**: a machine with more cores than agents' builds need could allow two. One is the safe start.
5. ~~**Non-Claude providers**~~ (checked 2026-10-24): the agent-turn path (Claude and the other persistent providers) already put the tool dirs first. The ACP and MSP controllers (`session_agent_env`) did not, so their agents never reached a shim; both now call `tool_store::prepend_agent_tools_path`, as the agent-turn path does. The terminal tier (Aider, Crush) runs as an ordinary shell pane, where the user's tools dir deliberately comes after the system PATH (`AfterExisting`), so its commands are not slotted; changing that would change every terminal pane, so it is left as it is.
