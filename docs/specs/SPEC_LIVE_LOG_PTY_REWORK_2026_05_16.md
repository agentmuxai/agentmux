# SPEC: Live-Log PTY Rework

**Status:** Draft — Phase α paused after first implementation attempt hit a deeper ConPTY issue than §8.1 predicted. See §12 for findings; the buffering diagnosis (§1–§5) and roadmap (§6–§7) remain valid.
**Date:** 2026-05-16
**Author:** AgentA
**Supersedes (in part):** [`SPEC_STREAMING_BASH_RUNNER_2026_05_11.md`](./SPEC_STREAMING_BASH_RUNNER_2026_05_11.md) §4.1 (transport)
**Related retros:** [`2026-05-11-live-log-streaming-wrapper-failures.md`](../retro/2026-05-11-live-log-streaming-wrapper-failures.md)
**Reference architecture:** Microsoft VS Code agent-mode terminal tool (open source — see §10)

---

## 1. Problem

The live-log feature (per `SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md`) is supposed to render each line of a bash tool's stdout/stderr as it is produced, so the user can watch the command run in real time inside the tool overlay.

It doesn't work. For most non-trivial commands, the overlay shows `⏳ Running...` for the entire duration of the command, and the whole output arrives in a single burst within ~40 ms at the end.

Diag instrumentation in PR #888 (v0.33.899) proved:

- The Solid reactive chain works correctly. When chunks reach the document atom, the partition memo, the Index per-position signal, and the ToolOverlayLog gates all re-evaluate as expected. There is no UI bug.
- The reducer receives the chunks correctly — 453 `tool-chunk-appended` events in the trace session.
- The chunks themselves arrive at the frontend in a single burst at the end of bash execution. Example timeline for one tool (toolu_01Jb1Xib, 3.6 s total):
  - `T+0.004 s` — chunk #1 (the bashwrap "starting" system marker)
  - `T+3.566 s` — chunks #2–#12 (11 stdout chunks) arrive within **53 ms**

Root cause: **stdio block-buffering when `isatty(STDOUT_FILENO) == 0`.** When a libc-stdio-using program (grep, awk, npm, cargo, python, node, ...) sees that its stdout is a pipe (not a terminal), glibc switches stdout from line-buffered (`_IOLBF`) to block-buffered (`_IOFBF`, ~4 KB). Output accumulates in the user-space stdio buffer until either the buffer fills or the program exits. The pipe-based `agentmux-bashwrap` design from PR γ (#809) cannot observe those intermediate writes — there are none until flush.

This is a structural property of the transport, not a bug in the wrapper or the reducer. No amount of frontend reactivity work will surface the missing bytes; they never leave the child process until EOF.

`stdbuf -oL` (attempted in PR #888) is a partial workaround on Linux/macOS via `LD_PRELOAD` of a shim that calls `setvbuf(stdout, _IOLBF)` during child startup. **Confirmed not effective on Windows Git Bash** — there's no `LD_PRELOAD` mechanism on Windows, so the binary is essentially a no-op there. Trace evidence: with the stdbuf wrap active (v0.33.899 with the bashwrap "spawning stdbuf -oL -eL bash -c (line-buffered)" log line firing), the same burst-at-end pattern persisted.

## 2. Background: how we got here

The β.A wrapper (PR #804) used `portable_pty` + `cmd /C` and shipped to smoke test. Every command failed with `STATUS_DLL_INIT_FAILED` (`0xC0000142`) on Windows. The root cause was a five-line ConPTY lifetime bug at `agentmux-bashwrap/src/bash_wrap.rs:253-257` (β.A version):

```rust
let mut reader = pair.master.try_clone_reader()?;
drop(pair.slave);   // OK
drop(pair.master);  // anti-pattern on Windows
```

On Windows, `pair.master` is the **pseudoconsole anchor**, not just a writer to the slave's stdin. Closing it during child startup tears down ConPTY mid-init and the child exits with `STATUS_DLL_INIT_FAILED` before producing a byte. The portable-pty maintainer documented this exact anti-pattern in [wezterm discussion #4674](https://github.com/wezterm/wezterm/discussions/4674).

The retro offered two fix paths:
1. **5-line fix** — keep `pair.master` alive across `child.wait()`. Preserves PTY semantics.
2. **Larger refactor** — drop PTY entirely, use `tokio::process::Command` + `Stdio::piped()` running `bash -c`.

We chose path 2 (PR γ, #809). The reasoning in `2026-05-11-...md` §4.2:

> *"The live-log feature wants line streaming, not spinner fidelity. PTY is a hedge against a problem we don't have, and it locks us into platform-specific PTY shenanigans."*

And in §5:

> *"npm install's `[==>] 50%` progress bar won't animate live, because npm checks `isatty(stdout)` and emits flat text when piped. For the log feature that's fine — the spinner/progress aesthetic is nice-to-have, line-streaming is the actual ask."*

The retro's lesson #4 codified the framing: **"PTY is a hedge, not a default."**

## 3. Why §2's framing was wrong

The retro treated `isatty()`-gated behavior as **cosmetic** — colors, progress bars, animated spinners — things you can lose without losing "line streaming."

That framing is incomplete. `isatty()` controls two distinct things at the same time:

1. **Whether the program emits decorative bytes** (ANSI color sequences, `\r`-overwriting progress bars, alt-screen apps like `less`). This is the cosmetic axis the retro identified.
2. **Whether libc switches stdout buffering from line-buffered to block-buffered.** This is *not* cosmetic — it's the difference between "flush on every `\n`" and "flush only when 4 KB accumulate or the process exits."

The retro reasoned about (1) and assumed (2) didn't exist or didn't matter. Today's data is the disproof: with pipes, every libc-stdio-using program block-buffers, and the live-log overlay sees zero intermediate output even when the command runs for seconds and produces dozens of lines.

The reason it took six months to notice: **bash builtins (`echo`, `printf`) are not affected by libc stdio buffering** — bash implements them via direct `write()` syscalls. Our isolated smoke tests used `bash -c 'for i in 1 2 3; do echo $i; sleep 1; done'` and observed correct line streaming. But Claude's bash tool overwhelmingly invokes *external programs* (`grep`, `git`, `npm`, `cargo`, ...), and those programs do go through libc stdio. The smoke test exercised the one case that doesn't reproduce the bug.

The correct retro lesson should have been:

> **PTY is the default for `isatty`-respecting agents. Pipes are a hedge against ConPTY lifetime bugs that we should retire by fixing the lifetime bug, not by abandoning PTY.**

## 4. Reference: how VS Code solves this

VS Code's agent-mode terminal tool (open source at `microsoft/vscode`) takes a position we should learn from:

| Dimension | VS Code | AgentMux today |
|---|---|---|
| Capture | Real PTY (node-pty) + xterm.js buffer | Plain `Stdio::piped()` |
| UI flow | Chat shows collapsible "Ran `xyz`" pill; click expands an embedded live xterm.js | Chat streams every stdout chunk into a Solid ChunkList in real time |
| Output sampling | Exponential-backoff polling of the xterm buffer between OSC 633 markers | Per-line dispatch from bashwrap → WPS → reducer |
| Shell integration | OSC 633 markers demarcate command start/end + exit code | None — no markers |
| Alt-buffer apps (vim/less/top) | Detected, replaced with stub message | Would corrupt the chunk list |
| Long-running commands | "Move to background" + idle-silence timeout; model uses `get_terminal_output`/`send_to_terminal` to interact | Run synchronously to EOF |

Key entry points in vscode for further reading:
- `src/vs/workbench/contrib/terminalContrib/chatAgentTools/browser/tools/runInTerminalTool.ts`
- `.../monitoring/outputMonitor.ts` (the polling FSM)
- `.../executeStrategy/{rich,basic,none}ExecuteStrategy.ts`

Our intent isn't to copy their UI choice (chat pill + pop-out xterm) — our overlay design predates that and is its own product decision. The architectural lesson is **PTY is non-negotiable for correct semantics**, and **OSC 633 markers are the right way to know when a command is done**, both of which we can adopt independently of the UI shape.

## 5. Goals

- **G1.** Live-streaming bash output works for **every** external command (`grep`, `npm`, `cargo`, `python`, `node`, ...) on Windows, Linux, and macOS, not just bash builtins.
- **G2.** No regression of the wedge that motivated the β.A → pipe rewrite: the wrapper's child must not fail with `STATUS_DLL_INIT_FAILED` on Windows.
- **G3.** Same public contract as today's bashwrap: hook subcommand input, `exec` argv shape, WPS publish payload, aggregated model_blob format.
- **G4.** Backwards compatible with existing tool overlay UI — the same `ToolChunkAppend` events flow into the reducer, the same `ChunkList` renders them.
- **G5.** No new dependency unless strictly required (we already had `portable_pty` in β.A; we'll bring it back).
- **G6.** Failure mode is degraded streaming, not failed command. If PTY allocation fails for any reason, fall back to plain pipes (today's behavior) so the command still runs and the user still gets the model-visible aggregated blob.

## 6. Non-goals

- **Not VS Code's UI.** No pop-out live xterm, no "Ran `xyz`" pill replacing the overlay. The overlay design is unchanged.
- **Not OSC 633 in this spec.** Shell-integration markers are valuable (Phase γ in the roadmap below) but layered on top of the PTY base; this spec ships the PTY base and defers markers.
- **Not full xterm.js in the frontend.** Phase β below mentions a *headless* xterm.js parser as an option for clean ANSI handling; making it user-visible is out of scope.
- **Not alt-buffer detection.** Vim/less/top inside a tool overlay is undefined behavior for now; if we encounter corruption, we strip alt-buffer escape sequences in Phase β rather than render the alt buffer. Out of scope for this spec.
- **Not progress-bar rendering.** Colored progress bars (`cargo build`, `npm install`) will be visible only as raw bytes / stripped text until Phase β. Not blocking.

## 7. Design

### 7.1 Phase α — PTY base layer (this PR)

Replace the `tokio::process::Command` + `Stdio::piped()` invocation in `agentmux-bashwrap/src/bash_wrap.rs::run_proc` with a `portable_pty` invocation that **keeps `pair.master` alive across `child.wait()`** (the β.A bug, now correctly fixed).

#### 7.1.1 Cargo dep

Re-add `portable-pty` to `agentmux-bashwrap/Cargo.toml`:

```toml
[dependencies]
portable-pty = "0.9"
```

(Same version `agentmux-srv` uses in `shell.rs`. Verify with `cargo tree -p agentmux-srv | grep portable-pty` and pin to the same.)

#### 7.1.2 `run_proc` rewrite — the critical lifetime contract

```rust
async fn run_proc(
    args: &Args,
    command: &str,
    wps: Option<&WpsClient>,
    buffered: Arc<Mutex<Vec<u8>>>,
) -> Result<i32> {
    let bash = locate_bash()?;
    let pty_system = native_pty_system();

    // Default PTY size — small but non-zero. We don't currently feed
    // SIGWINCH on host window resize; that lands with the headless-
    // xterm work in Phase β. 80x24 is the safe default that most
    // programs assume.
    let pair = pty_system.openpty(PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    }).context("openpty")?;

    // Build the child command. NOTE: we no longer wrap with stdbuf —
    // the PTY makes the child see isatty=1, which auto-selects
    // line-buffered stdio, which is exactly what we wanted from
    // stdbuf. Two layers of "fix the buffering" is redundant.
    let mut cmd = CommandBuilder::new(bash.as_os_str());
    cmd.arg("-c");
    cmd.arg(command);
    if let Some(cwd) = std::env::var_os("PWD") {
        cmd.cwd(cwd);
    }

    // CRITICAL: spawn the child via the SLAVE side. The master side
    // stays in scope here (and is held across child.wait() below) —
    // dropping it during child startup is what produced the β.A
    // STATUS_DLL_INIT_FAILED bug. See wezterm discussion #4674 and
    // retro 2026-05-11-live-log-streaming-wrapper-failures.md §4.2.
    let mut child = pair.slave.spawn_command(cmd)
        .with_context(|| format!("spawning bash at {}", bash.display()))?;
    // Slave handle is no longer needed in the parent now that the
    // child owns its end. Master MUST stay alive.
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().context("clone reader")?;

    // The reader is sync (portable_pty wraps a synchronous handle).
    // Move it into a blocking task to avoid blocking the tokio
    // worker; relay LineEvents through an mpsc to the publisher loop.
    let (tx, mut rx) = mpsc::channel::<LineEvent>(1024);
    let tx_stdout = tx.clone();
    // PTY merges stdout and stderr by default (single read stream).
    // We tag all output as "stdout" — distinguishing stderr would
    // require OSC 633 markers or a second PTY (deferred).
    tokio::task::spawn_blocking(move || {
        run_pty_reader(reader, "stdout", tx_stdout);
    });
    drop(tx);

    // Publisher loop — same as before.
    let tool_id = args.tool_id.clone();
    let block_id = args.block_id.clone();
    let wps_clone = wps.cloned();
    let buffered_clone = buffered.clone();
    let publisher_handle = tokio::spawn(async move {
        publisher_loop(rx, tool_id, block_id, wps_clone, buffered_clone).await;
    });

    // Wait for the child. CRITICAL: `pair.master` must still be in
    // scope here on Windows ConPTY. Don't restructure this fn to
    // return early before child.wait() completes — see retro §4.2.
    let exit_status = tokio::task::spawn_blocking(move || child.wait())
        .await
        .context("wait join")?
        .context("child wait")?;
    let _ = publisher_handle.await;

    // master drops here, after the child is reaped — safe ordering.
    drop(pair.master);

    Ok(exit_status.exit_code() as i32)
}

fn run_pty_reader(
    mut reader: Box<dyn std::io::Read + Send>,
    kind: &'static str,
    tx: mpsc::Sender<LineEvent>,
) {
    use std::io::Read;
    let mut pending: Vec<u8> = Vec::with_capacity(8192);
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => {
                if !pending.is_empty() {
                    let _ = tx.blocking_send(LineEvent { kind, bytes: std::mem::take(&mut pending) });
                }
                return;
            }
            Ok(n) => {
                pending.extend_from_slice(&buf[..n]);
                while let Some(nl_pos) = pending.iter().position(|&b| b == b'\n') {
                    let line: Vec<u8> = pending.drain(..=nl_pos).collect();
                    if tx.blocking_send(LineEvent { kind, bytes: line }).is_err() {
                        return;
                    }
                }
                // Newline-free residue past FLUSH_BYTES — same flush
                // policy as the pipe version.
                if pending.len() >= FLUSH_BYTES {
                    if tx.blocking_send(LineEvent {
                        kind,
                        bytes: std::mem::take(&mut pending),
                    }).is_err() {
                        return;
                    }
                }
            }
            Err(e) => {
                tracing::warn!(target: "bashwrap", error = %e, kind, "pty read error");
                return;
            }
        }
    }
}
```

The publisher loop and the model_blob aggregation stay byte-for-byte the same as today. The wire shape, the WPS event name, the `op: chunk | terminal` payload — all unchanged.

#### 7.1.3 Fallback path (G6)

```rust
match pty_system.openpty(PtySize { ... }) {
    Ok(pair) => { /* PTY path above */ }
    Err(e) => {
        tracing::warn!(target: "bashwrap", error = %e, "PTY allocation failed — falling back to pipes");
        run_proc_via_pipes(args, command, wps, buffered).await
    }
}
```

`run_proc_via_pipes` is today's `run_proc` body, extracted intact (minus the stdbuf wrap, which is reverted as part of this PR). It exists strictly as the failure-mode safety net.

#### 7.1.4 What to delete

- The `stdbuf -oL -eL` wrap added in PR #888 (`agentmux-bashwrap/src/bash_wrap.rs::run_proc`). PTY makes it redundant; keeping both is two implementations of the same fix.
- The "spawning bash -c (stdbuf not found — output may buffer)" log branch. With PTY as the default and pipes as the fallback, the relevant log lines become "PTY mode" vs "pipe fallback after PTY allocation failed."

### 7.2 Phase β — ANSI normalization (follow-up, not this spec)

PTY output contains raw ANSI escape sequences: colors (`\x1b[31m`), cursor moves (`\x1b[2A`), line clears (`\x1b[K`), alt-buffer enter/exit (`\x1b[?1049h`), and OSC sequences. The current `ChunkList` renders each chunk as a `<pre>` text node, so escape sequences will appear as literal `^[[31m` garbage.

Two options for the normalization step (decided in the Phase β spec):

1. **Server-side strip** — strip ANSI sequences in bashwrap before publishing. Loses colors but keeps the chunk-list UI unchanged. Smallest diff.
2. **Headless xterm.js parser** — push raw bytes to the frontend; feed them into an off-screen xterm.js instance; render the cooked screen text. Preserves colors (via xterm's serialize addon), handles alt-buffer correctly, and matches VS Code's architecture. Bigger lift but the right destination.

This spec ships PTY with raw bytes flowing through unchanged. Phase β will add the normalization layer.

### 7.3 Phase γ — OSC 633 shell integration (further follow-up)

After Phase β stabilizes, install a bash-rc fragment that emits OSC 633 markers around each command Claude runs:

```bash
PROMPT_COMMAND='printf "\e]633;D;%s\a" $?; PROMPT_COMMAND_ORIG'
PS1=$'\e]633;A\a'"$PS1"$'\e]633;B\a'
preexec () { printf "\e]633;C\a"; }
```

With markers in place, bashwrap can:
- Detect command-finished without depending on the child exiting (long-running commands move to background).
- Demarcate exactly which bytes belong to which tool_use (today we already have that via per-bashwrap-process scoping, so this is more useful when one bash runs multiple Claude tool_use calls in sequence).
- Surface exit code via the marker payload rather than the process exit code.

Out of scope for the PTY rework. Documented here so future-us doesn't redesign the protocol from scratch.

## 8. Risk and mitigation

### 8.1 Re-introducing the β.A wedge

**Risk:** the master-handle lifetime contract is the exact bug that wedged β.A. If `pair.master` falls out of scope before `child.wait()` returns, every Windows command fails with `STATUS_DLL_INIT_FAILED` again.

**Mitigation:**
- §7.1.2 above keeps `pair.master` in `run_proc`'s local scope across the `child.wait()` await.
- An end-to-end smoke test (§9) runs the same `echo hello` command that wedged β.A as the first acceptance gate.
- Code review: the comment on the `drop(pair.master)` line at the end of `run_proc` explicitly references the retro file so any future maintainer who tries to move it earlier will see the warning.
- Unit test: a `run_proc_pty_master_alive_during_wait` test that stubs the child wait to a slow future and asserts `pair.master` is still readable at that point. (Can't trigger the bug deterministically without ConPTY in the test environment, but the test pins the assumption.)

### 8.2 PTY merges stdout and stderr

**Risk:** with PTY the child's stderr is merged into the master read stream. Today we tag chunks `kind: "stdout" | "stderr"`. Phase α tags everything `"stdout"` (see §7.1.2).

**Mitigation:** the chunk renderer doesn't currently style stderr differently (`KIND_CLASS` in `ToolOverlayLog.tsx` defines a `--stderr` variant but nothing in the live trace exercises it visibly), so this is cosmetic. If needed, a second PTY for stderr is possible but doubles ConPTY lifetime risk; OSC 633 markers (Phase γ) are the cleaner separator. **Accept the limitation in α.**

### 8.3 PTY default size + SIGWINCH

**Risk:** opening at 80x24 means programs like `cargo build` will format progress bars for an 80-column terminal even if the user's overlay is wider. No SIGWINCH wiring.

**Mitigation:** acceptable for α (the overlay doesn't actually render terminal-shaped output — see Phase β). Phase β with xterm.js will resize the headless terminal to match the overlay width and emit `ioctl(TIOCSWINSZ)` accordingly.

### 8.4 Cross-platform parity

**Risk:** `portable_pty` covers Win32 ConPTY, macOS, and Linux, but the API has platform-specific quirks (e.g., Windows requires `cmd.cwd` set to a real directory or spawn fails; Unix doesn't care).

**Mitigation:**
- §7.1.2 sets `cwd` from `$PWD` (always present on both platforms inside a Claude subprocess).
- The fallback path (§7.1.3) handles the case where PTY allocation fails for any reason, including platform-specific quirks we don't predict.
- CI: run the smoke command on all three platforms in `task test` (we already have a matrix; add a `cargo test -p agentmux-bashwrap pty_smoke_test`).

### 8.5 Performance — many small reads

**Risk:** PTY read loop runs in a blocking thread (`tokio::task::spawn_blocking`). For very chatty commands (`yes`, infinite loops), the blocking task pool could saturate.

**Mitigation:**
- One blocking task per active bashwrap process; we don't run hundreds in parallel.
- The existing `FLUSH_BYTES` size guard (4 KB) caps memory.
- If we observe pool saturation in practice, switch the reader to async via `tokio::fs::File::from_std` of the raw handle on Unix and `tokio::io::Bidirectional` on Windows. Premature for α.

## 9. Test plan

### 9.1 Wedge-regression smoke

The β.A failure mode was "every Windows command fails." First gate after this PR builds:

```bash
agentmux-bashwrap exec --tool-id=test --b64-cmd=$(printf 'echo hello' | base64)
# Expect: stdout contains "hello", exit code 0.
# Reject if exit code is -1073741502 (0xC0000142).
```

If this smoke fails, the PR is wedged exactly like β.A and the master-handle fix didn't take. Diagnose before moving on.

### 9.2 External-command streaming

A bash command that exercises the original symptom — external program with libc stdio that takes >1 s and emits lines over time:

```bash
agentmux-bashwrap exec --tool-id=test --b64-cmd=$(printf 'for i in 1 2 3 4 5; do date; sleep 1; done' | base64)
```

`date` is an external program in `coreutils` that goes through libc stdio. Expectation: five WPS chunks land at the broker spaced ~1 s apart (not five chunks in a burst at end). This is the empirical proof that the buffering issue is fixed.

Trace-level verification: capture `[live-log-diag] tool_chunk received` log lines and assert timestamps are spread across the run, not bunched within the last 100 ms.

### 9.3 Real Claude flow

End-to-end smoke inside an agent pane:

1. Build `task package:local`, launch the portable.
2. Open a fresh agent pane, load Maks (or any continue agent).
3. Prompt the agent to run `for i in 1 2 3 4 5; do echo "line $i"; date; sleep 1; done`.
4. Expand the tool overlay while the command is running.
5. Watch lines append one at a time at ~1 s intervals.

Acceptance: the overlay shows lines appearing live, not in a single burst at the end.

### 9.4 Fallback verification

Force PTY allocation to fail (set `portable_pty`'s test hook, or simulate by spawning in a context without ConPTY) and verify the wrapper falls back to pipes without crashing. Less critical than 9.1–9.3; can defer to a follow-up if it adds review surface area.

## 10. Open questions

- **Q1.** Do we need a `RUST_LOG=agentmux_bashwrap=debug` tracing level for the master-handle lifetime, or is the existing `info` level enough? Adding a `pty_lifetime_debug` env var that explicitly logs each master/slave handle transition would help future-us debug ConPTY regressions without rebuilding. Default off.

- **Q2.** Is there a path to surface PTY-only features (colors, progress bars) into the overlay without committing to a full headless xterm.js? A minimal ANSI-color → CSS-class transformer in the chunk renderer could buy us colorized output for ~50 lines of code, deferring the full xterm.js parser to a later phase. Worth exploring as part of Phase β scoping.

- **Q3.** Should we re-instate the `kind: "stderr"` distinction at all, given Phase α merges streams? OSC 633 has a `;E` marker for command-error output, which would let us tag stderr-after-stdout-on-same-line correctly. But this is Phase γ territory; for now everything is `kind: "stdout"`.

- **Q4.** PTY size — 80x24 is the safe default but might cause progress-bar artifacts. Should we expose `AGENTMUX_PTY_COLS` / `AGENTMUX_PTY_ROWS` env vars so the host can pass a sensible default based on the user's overlay width? Trivial to add; deferring to Phase β makes the dynamic resize and the env-driven default land together.

## 11. References

### Code locations (current state, pre-rework)

- Wrapper run loop: [`agentmux-bashwrap/src/bash_wrap.rs::run_proc`](../../agentmux-bashwrap/src/bash_wrap.rs) (lines ~397–500, current pipe + stdbuf form)
- WPS publish client: [`agentmux-bashwrap/src/wps_client.rs`](../../agentmux-bashwrap/src/wps_client.rs) (unchanged by this rework)
- Frontend tool_chunk handler: [`frontend/app/view/agent/useAgentStream.ts`](../../frontend/app/view/agent/useAgentStream.ts) (`blockChunkUnsub` block ~lines 109–170)
- Reducer ToolChunkAppend: [`frontend/app/store/agent-document/reducer.ts`](../../frontend/app/store/agent-document/reducer.ts) (lines ~202–252)
- Tool overlay UI: [`frontend/app/view/agent/components/ToolOverlayLog.tsx`](../../frontend/app/view/agent/components/ToolOverlayLog.tsx) (unchanged by this rework)
- Working PTY consumer (reference implementation): [`agentmux-srv/src/backend/shell.rs`](../../agentmux-srv/src/backend/shell.rs) — `pair.master` lifetime handled correctly today

### External references

- [wezterm discussion #4674](https://github.com/wezterm/wezterm/discussions/4674) — portable-pty maintainer on `STATUS_DLL_INIT_FAILED` root cause
- [wezterm issue #4206](https://github.com/wezterm/wezterm/issues/4206) — Windows-specific portable-pty pitfalls
- [VS Code `runInTerminalTool.ts`](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/contrib/terminalContrib/chatAgentTools/browser/tools/runInTerminalTool.ts) — entry point of VS Code's agent terminal tool
- [VS Code `outputMonitor.ts`](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/contrib/terminalContrib/chatAgentTools/browser/tools/monitoring/outputMonitor.ts) — polling FSM
- [VS Code `richExecuteStrategy.ts`](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/contrib/terminalContrib/chatAgentTools/browser/executeStrategy/richExecuteStrategy.ts) — OSC 633 shell-integration strategy
- [Terminal Integration (DeepWiki)](https://deepwiki.com/microsoft/vscode-copilot-chat/7.4-terminal-integration) — VS Code's terminal architecture overview
- [glibc stdio buffering](https://www.gnu.org/software/libc/manual/html_node/Buffering-Concepts.html) — the `_IOFBF`/`_IOLBF`/`_IONBF` mode selection

## 12. Implementation findings (Phase α attempt, 2026-05-16)

We attempted Phase α (§7.1) in this session. The implementation matched the §7.1.2 sketch as closely as Rust syntax allowed — `portable-pty = "0.9"` added to `agentmux-bashwrap/Cargo.toml`, `run_proc` rewritten with the PTY-default-pipe-fallback structure, master-handle lifetime preserved through `tokio::task::spawn_blocking`. It compiled clean.

**Smoke 9.1 (`echo hello`) hung indefinitely.** Trace from `eprintln!` instrumentation:

```
[bashwrap-trace] run_via_pty: entering
[bashwrap-trace] spawn_command done
[bashwrap-trace] try_clone_reader done
[bashwrap-trace] publisher spawned
[bashwrap-trace] reader task starting
[bashwrap-trace] wait task: calling child.wait()
<hung for 5+ seconds, killed with SIGKILL>
```

`child.wait()` never returned, even though the child was `bash -c "echo hello"` which should complete in microseconds. The reader task also never logged a single `read()` returning — no bytes ever arrived at the master. Master-handle lifetime was correct (the entire `PtyPair` was moved into the wait task's closure, satisfying the ConPTY contract).

Two variants tried in this session, both wedged the same way:

1. **Drop slave after spawn** (initial attempt) — destructured `PtyPair { slave, master }`, called `slave.spawn_command(cmd)`, then `drop(slave)`. Hung.
2. **Hold whole pair in wait task** (retry, matching `agentmux-srv/src/backend/blockcontroller/shell.rs`) — kept `pair` in scope and moved the whole struct into the spawn_blocking closure. Hung the same way.

So the §8.1 risk ("re-introducing the β.A wedge") is more subtle than the spec predicted. Holding the master across `child.wait()` is necessary but **not sufficient** for ConPTY to work in this wrapper context. Something else is keeping the child from progressing, the master read end from receiving bytes, or `child.wait()` from observing exit. Possible directions to investigate next:

- **`take_writer` may be required.** `agentmux-srv/src/backend/blockcontroller/shell.rs:746` calls `pair.master.take_writer()` even for read-only use. We omitted that call (we don't inject stdin). ConPTY may require the writer to be "claimed" even if unused — without it the pseudoconsole may not advance the child's I/O. Try adding `let _writer = pair.master.take_writer()?;` and holding it across the wait.
- **Async runtime interaction with `spawn_blocking`.** `child.wait()` from `portable_pty` is sync. Running it inside `tokio::task::spawn_blocking` may interact badly with how ConPTY's underlying handle wait is satisfied. A bare `std::thread::spawn` for the wait, with a sync channel back to the async caller, may behave differently. Worth A/B-testing.
- **Reader has to be actively draining before the child can progress.** The trace shows the reader task starts but never observes a read return. If ConPTY's pipe buffer is small and the child blocks on its very first write, the reader needs to be a few microseconds ahead of the child. The current ordering (`spawn_blocking(reader)` immediately before `spawn_blocking(wait)`) leaves the scheduling order to tokio's whim. Explicit `std::thread::spawn` for the reader (which starts running synchronously) may close this race.
- **Different child invocation.** Try `cmd /C bash -c "echo hello"` instead of bash directly. cmd may handle PTY teardown more gracefully — or might not, but it's a one-line test that distinguishes "bash-specific PTY issue" from "general ConPTY-via-portable_pty issue."
- **Different PTY backend.** The portable-pty crate uses Microsoft's ConPTY API on Windows. Alternative wrappers (e.g. `conpty` crate directly, or `windows-rs`'s `CreatePseudoConsole`) may give us finer control. Bigger lift.

**What we did NOT yet try:** A literal copy of `agentmux-srv/src/backend/blockcontroller/shell.rs` lines 395–660 into a standalone Rust binary that does only "spawn echo, capture output, exit" — proving whether the shell.rs pattern works in a short-lived process at all. If it does, the diff against our wrapper code identifies the broken assumption. If it doesn't, the issue is shell.rs's pattern relying on its long-lived task structure to work, and the wrapper needs something genuinely different.

**Outcome (initial attempt):** the first implementation attempt was reverted (Cargo.toml and bash_wrap.rs restored to pre-PR state via `git checkout`). The recommendation below was followed: a standalone V1/V2 repro was built, which identified DSR as the missing piece.

**Resolution (this PR):** PR #888 *does* ship the PTY path. The wedge's root cause turned out to be the DSR (Device Status Report) handshake: bash emits `\x1b[6n` at startup and blocks on stdin waiting for the cursor-position reply. A standalone V1 repro reproduced the hang; V2 added a DSR responder that writes `\x1b[1;1R` back through the master writer and the hang disappeared. That responder is now in `pty_reader_loop` / `strip_and_answer_dsr`. The wrapper additionally manually prepends `/usr/bin`, `/usr/local/bin`, and `/mingw64/bin` to PATH instead of using `bash -l`, so cold start stays ~150ms.

**Recommendation (historical, applied):** before another attempt, build the standalone repro (the §12 "did NOT yet try" item) and validate the shell.rs pattern in a short-lived process. That isolates the question — is the issue our wrapper code, or the spawn-via-portable_pty-and-wait-immediately pattern itself? — with much smaller surface area than a full bashwrap PR.

If the repro works, port the exact pattern into `run_via_pty`. If it doesn't, the answer is to skip portable_pty for this use case and either (a) use the `conpty` crate directly with tighter control over the lifecycle, or (b) keep pipes for the wrapper but install a bash-rc snippet via the AGENTMUX hook that calls `stty` to fake TTY-ness for child commands (a hack but cross-platform).

---

### AgentMux internal references

- Retro: [`docs/retro/2026-05-11-live-log-streaming-wrapper-failures.md`](../retro/2026-05-11-live-log-streaming-wrapper-failures.md) — full context of the β.A wedge and the pipe rewrite
- Analysis: [`docs/analysis/TOOL_OUTPUT_STREAMING_2026_05_11.md`](../analysis/TOOL_OUTPUT_STREAMING_2026_05_11.md) — original design exploration
- Spec (transport, superseded in part): [`docs/specs/SPEC_STREAMING_BASH_RUNNER_2026_05_11.md`](./SPEC_STREAMING_BASH_RUNNER_2026_05_11.md)
- Spec (UI, unaffected): [`docs/specs/SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md`](./SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md)
- Workflows discussion (long-term streaming tracking): GitHub Discussion #832 (agentmuxai/agentmux)
