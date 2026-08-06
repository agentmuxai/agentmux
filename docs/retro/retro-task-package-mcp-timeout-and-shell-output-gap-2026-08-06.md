# Retro: `task package` unbuildable via agent tooling — Bash-call timeout, `nohup` doesn't detach, MCP Shell output unreadable

**Date:** 2026-08-06
**Severity:** Medium — blocked an agent-driven release build; user had to be asked how to proceed. No data loss, no incorrect merge (the code-side work — 3 PRs — completed correctly; only the *build artifact* step failed).
**Observed by:** agenta (Claude agent) while producing a fresh desktop portable for v0.54.11
**Related retros:** `retro-task-dev-agent-shell-path-2026-06-27.md` (same *symptom signature*, different actual cause — see Historical Chain), `retro-agentmux-srv-9min-crash-2026-07-26.md` (independent, still-open suspicion of Windows Job Object lifecycle issues)

---

## TL;DR

Producing a `task package` build (compiles the CEF-linked `agentmux-cef` host binary, then bundles/packages it — the heaviest build target in this repo) failed six consecutive times across three different invocation mechanisms:

1. **3× via the Bash tool**, each dying with `task: Signal received: terminated` at almost the identical point (right after `agentmux-cef (lib)` finishes compiling, before the host *binary* link step completes) — even with an active heartbeat proving `agentmux-bashwrap`'s documented 600s idle-timeout was NOT the trigger.
2. **1× via `nohup ... & disown`** from inside a Bash tool call, attempting to escape the tool call's own lifetime — the output log came back empty and the process was gone almost immediately, meaning the detach did not survive.
3. **2× via `mcp__agentmux__Shell`** — both exited near-instantly (`exit_code: 1`, 1–2 lines) with **no way to retrieve the actual output content** through any available tool.

The eventual fix that worked: `scripts/package-agent.cmd` (a bridge script that fixes exactly this class of problem) **already exists in this repo** and would have avoided both MCP Shell failures — but it is referenced nowhere except itself, so it wasn't found until an explicit source-code investigation turned it up.

---

## What Happened

### Attempts 1–3 — Bash tool, `task package` (with and without a heartbeat)

**Attempt 1** (no heartbeat): failed after real progress (compiled `agentmux-launcher`, `agentmux-cef` lib, many warnings printed) with:
```
task: Signal received: "terminated"
task: Failed to run task "build:host": task: Failed to run task "build:host:windows": exit status 58
```

Initial hypothesis: `agentmux-bashwrap`'s documented idle-kill guard (600s of zero PTY output — see CLAUDE.md and `agentmux-bashwrap/src/bash_wrap.rs:294`) fired during a long, silent linker stretch.

**Attempt 2** (with a `while true; do sleep 120; echo heartbeat; done &` backgrounded alongside `task package`, specifically to defeat that idle-timeout): heartbeat lines appeared in the log on schedule (`11:46:23`, `11:48:23`) — proving bytes *were* actively flowing to the PTY — and the build **still** died with the identical `Signal received: "terminated"` message, at essentially the same relative point in the build.

**Attempt 3** (same heartbeat pattern, retried to benefit from a warmer cargo cache from attempts 1–2): identical outcome, identical failure point (`agentmux-cef (lib) generated 51 warnings` → heartbeat → heartbeat → terminated).

**Diagnosis (post-hoc source read, `agentmux-bashwrap/src/bash_wrap.rs`):**
- The idle-timeout is confirmed to be a strict zero-bytes-for-600s timer (lines 281–302, 997–1055) — attempt 2/3's flowing heartbeat rules it out definitively.
- No separate overall wall-clock cap exists anywhere in `agentmux-bashwrap`'s source (`bash_wrap.rs`, `main.rs`, `hook.rs`, `wps_client.rs`) — every `Duration`/timeout in that crate is either the 600s idle timer or a short (≤5s) shutdown grace period.
- `task: Signal received: "terminated"` is go-task's own `signal.Notify` handler firing — on Windows this responds to a **console control event** (`CTRL_C_EVENT`/`CTRL_BREAK_EVENT`/`CTRL_CLOSE_EVENT`), not an unconditional `TerminateProcess`/`TerminateJobObject` (those give the target zero chance to run code, so it could never have printed this message). This narrows the search to "something delivered a console-control event to the whole ConPTY," not a hard external kill.
- **Best-supported (not proven) hypothesis:** the Claude Code CLI itself enforces its own Bash-tool timeout (`BASH_DEFAULT_TIMEOUT_MS`/`BASH_MAX_TIMEOUT_MS`), independent of anything in this repo, and delivers exactly this kind of console-control teardown when it fires. This lives outside `agentmuxai/agentmux` entirely — **not fixable from this codebase**, only designable-around.

### Attempt 4 — `nohup task package > log 2>&1 < /dev/null & disown`

Intended to fully detach the build so it would survive past the wrapping Bash tool call's own completion. Result: the log file was **0 bytes** and the process was gone within moments of the wrapping call returning.

**Diagnosis:** confirmed (unrelated code path, but a real, general mechanism on this platform) — `agentmux-srv/src/backend/process_tracker/windows.rs`'s `JobObjectTracker` assigns each agent pane's whole process tree to one Windows Job Object created **without** `JOB_OBJECT_LIMIT_BREAKAWAY_OK` (lines 83–85, deliberately: *"descendants can't opt out of the job... we want those attempts to fail so the child stays tracked"*). Job Object membership is inherited at OS process-creation time and is **not** something POSIX-level `nohup`/`disown`/`&` can opt out of on Windows — those are bash-level constructs with no Windows equivalent effect. Whether *this specific* job is what tore down attempt 4 is not proven (no code path was found that would trip `kill_tree()` mid-build for one continuous active session), but it confirms `nohup`-style detachment is structurally unreliable here in general, not just an unlucky one-off.

### Attempts 5–6 — `mcp__agentmux__Shell`

**Attempt 5:** `cmd: "cd /c/Users/area54/.../agentmux-wt-muxspect-dock && task package"` — exited in `exit_code: 1`, 1 line.

**Attempt 6:** `cmd: "cmd /C \"cd /d C:\\...&& set PATH=...Git\\bin...&& task package 2>&1\""` — exited in `exit_code: 1`, 2 lines.

Neither exposed its actual stdout/stderr through any tool available: `ShellStatus` returns only `running`/`exit_code`/`line_count`; the app's `Layout` showed no new pane; grepping the srv log around the exact `shell.create`/`shell.exit` timestamps found only tracing metadata, never content.

**Diagnosis (post-hoc source read, `agentmux-srv/src/backend/shell_node.rs` + `server/mod.rs`):**
- On Windows, `mcp__agentmux__Shell` spawns via `tokio::process::Command::new("cmd").args(["/C", &self.cmd])` **server-side** — no bash/MSYS2 layer, no PTY (`shell_node.rs`'s module doc: *"Phase 2... no PTY in this phase"*).
- Attempt 5's failure: **not** Gap A/B from the June 27 retro (there's no bash layer here to have that gap) — it was a **Unix-style path** (`/c/Users/...`) handed directly to `cmd /C`, which `cmd.exe` cannot parse.
- Attempt 6's failure: the `cmd` argument passed to `Shell()` already contained its own `cmd /C "..."` wrapper, and `ShellNodeRunner` wraps *every* command in `cmd /C` itself — the result was a **nested `cmd.exe` invocation with embedded quotes**, a distinct and well-known Windows quoting failure.
- **Confirmed, real product gap:** there is no RPC/REST path anywhere in this codebase to retrieve a shell's persisted output after the fact. `publish_chunk`/`publish_exit` (`shell_node.rs:528–668`) only broadcast live to a WPS scope (`shell:<shell_id>`) that the *frontend* subscribes to and renders inline in the calling agent's own pane — an agent reading its own tool results back has no equivalent read path. `/api/v1/shell/status` (`ShellStatusInfo`) never carries content, only `running`/`exit_code`/`line_count`.

---

## Historical Chain

| Retro | Date | Problem | Fix |
|-------|------|---------|-----|
| `retro-task-dev-agent-shell-path-2026-06-27.md` | Jun 2026 | `task dev` via MCP Shell: MSYS2 bash ignores `.cmd`, cmd.exe PATH missing `bash.exe` | Documented correct invocation; recommended a `.cmd` bridge wrapper as a longer-term follow-up |
| `retro-agentmux-srv-9min-crash-2026-07-26.md` | Jul 2026 | `agentmux-srv` dies ~9m37s into idle instances, no crash evidence | Still open — "current working theory (unproven)" names Windows Job Object teardown as leading suspect |
| **This retro** | Aug 2026 | `task package` unbuildable via any agent-tool mechanism: Bash-call timeout (external, unfixable here), `nohup` doesn't detach (Job Object semantics), MCP Shell output is unreadable (confirmed product gap) | See Fix Plan below |

The June 27 retro's recommended follow-up — *"Add a `task dev:agent` wrapper... a thin `.cmd` or PowerShell script that prepends `Git\bin`"* — **was actually built**, for both `task dev` (`scripts/dev-agent.cmd`) and `task package` (`scripts/package-agent.cmd`). `dev-agent.cmd` is documented in CLAUDE.md ("Launching `task dev` from an agent / MCP Shell (Windows)"). **`package-agent.cmd` has no equivalent documentation anywhere and is referenced nowhere except its own file** — which is almost certainly why attempts 5–6 improvised broken inline commands instead of using it. The fix from the last retro shipped; the doc update that would have made it discoverable didn't.

---

## Fix Plan

### Immediate (this session)

1. Use `scripts/package-agent.cmd` directly as the MCP Shell `cmd` argument (not wrapped in an additional `cmd /C "..."`), from the repo-relative path — this sidesteps both attempt-5/6 mistakes.
2. Redirect the command's own output to a file (`... > C:\...\pkg.log 2>&1`) so the *build's own shell redirection* — not `ShellStatus` — is the read path, then poll `ShellStatus` for `running: false` and `Read` the log file directly. This works around the confirmed "no output retrieval" gap without needing a product fix first.

### Documentation (do now, low-risk, high-value)

3. Add a **"Launching `task package` from an agent / MCP Shell (Windows)"** section to CLAUDE.md, mirroring the existing `task dev` section exactly, pointing at `scripts/package-agent.cmd`. This is the single highest-leverage fix here: the correct tool already existed and this retro would not have needed attempts 5–6 at all if it had been documented like its sibling.
4. Cross-link `package-agent.cmd`'s own header comment to CLAUDE.md (currently says "see dev-agent.cmd for full explanation" but not vice versa) so either file leads a future reader to the other.

### Product gap (real, worth fixing, not urgent)

5. **`mcp__agentmux__Shell`'s output is genuinely unretrievable after the fact.** Two options, not mutually exclusive:
   - Extend `ShellStatusInfo`/`/api/v1/shell/status` to optionally return the persisted chunk ring's content (it already exists server-side at `persist: 1024` for the WPS broadcast — the data isn't gone, just not exposed via this path).
   - At minimum, update the `Shell`/`ShellStatus` tool descriptions to explicitly warn: *"output is not retrievable via ShellStatus — redirect the command's own stdout/stderr to a file and read that file separately if you need to inspect output after the fact."* This is a one-line doc fix that would have prevented real confusion this session (I spent real effort trying to find output that structurally cannot be read this way).

### Standing limitation (design around, don't attempt to fix here)

6. **The Bash tool's ~10-minute-class timeout is not part of this repo** (best evidence points at the Claude Code CLI itself, via `BASH_DEFAULT_TIMEOUT_MS`/`BASH_MAX_TIMEOUT_MS`, independent of `agentmux-bashwrap`). Document in CLAUDE.md, near the `task package` section, that **any single build step expected to exceed ~10 minutes wall-clock must go through `package-agent.cmd`/`dev-agent.cmd` via MCP Shell with output redirected to a file** — the Bash tool (even backgrounded, even with a heartbeat, even with `nohup`) is not a reliable mechanism for a build this long on this platform, and that's expected, not a bug to chase further.

---

## What Worked, For The Record

**Confirmed, not just theorized:** invoking `scripts/package-agent.cmd` directly (no extra `cmd /C` wrapper) via `mcp__agentmux__Shell`, with its own output redirected to a file (`package-agent.cmd > C:\...\pkg-build.log 2>&1`), produced a shell that `ShellStatus` reported as genuinely `running` (not an instant `exit_code: 1`) and whose progress was readable in real time via a plain `Read` on the log file — `npm:install` skip, `build:frontend` (vite) running, real compiler/bundler output flowing. This validates both fix-plan items 1–2 in one shot: the correct wrapper script plus file-redirection-as-output-workaround is a fully working path for a build this long, on the first attempt using it.
