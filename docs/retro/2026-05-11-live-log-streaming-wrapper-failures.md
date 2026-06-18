# Retro — live-log streaming, the wrapper that didn't work

**Date:** 2026-05-11
**Owner:** AgentA
**Surface area:** `agentmux-bashwrap`, `agentmux-srv` (websocket + agent_config), `frontend/app/view/agent` log overlay
**PR sequence:** #800 → #803 → #804 → #808 (replaceChild hotfix) → #813 (settings.json hotfix) → **next** (wrapper rewrite)
**Status:** Streaming still doesn't work end-to-end at 0.33.808. This retro is being written while the wrapper rewrite (PR γ) is being planned.

---

## 0. TL;DR

We designed a sound architecture (analysis doc → spec → 3 PRs) but the implementation deviated from the design in two ways that we didn't flag at the time, and the wrapper binary that resulted has three independent bugs:

1. **Wrong hook discovery file.** β.B wrote `.claude/hooks.json`; Claude Code only reads `.claude/settings.json` under a `"hooks"` key. Fixed in PR #813.
2. **Wrapper's spawned child dies before producing output.** `portable_pty::CommandBuilder("cmd").args(["/C", cmd])` returns child exit code `-1073741502` (`0xC0000142`, `STATUS_DLL_INIT_FAILED`) on Win10 19045 for *every* command, including `echo hello`. **Root cause confirmed via research:** the wrapper drops `pair.master` immediately after cloning the reader; per the portable-pty maintainer, ConPTY can't tolerate that — dropping the master mid-startup is the canonical anti-pattern that produces exactly this error code. The same `CommandBuilder` pattern works in `agentmux-srv/.../shell.rs` because there `master` is moved into a long-lived task.
3. **Env vars don't propagate.** The wrapper logs `streaming disabled (auth/url env missing)` when invoked inside Claude — meaning `AGENTMUX_AUTH_KEY` and `AGENTMUX_LOCAL_URL` aren't reaching it even when websocket.rs's `agent_input` handler explicitly injects them for the Claude spawn. Unverified which of the five spawn hops drops them; instrumentation in the rewrite will tell us.

The new theory: drop `portable_pty` and `cmd /C` entirely from the wrapper; use `tokio::process::Command` with `Stdio::piped()` running `bash -c`. Plain pipes give us line streaming, which is all the live-log feature actually needs, and they sidestep the ConPTY lifetime footgun entirely. We could alternatively keep PTY and fix only the `master`-drop bug (one-line diff), but the costs of carrying portable_pty in a binary that doesn't need TTY semantics aren't worth the spinner fidelity. **Online research validates this direction:** the wezterm maintainer's recommendation for stream-capture use cases is explicitly "use plain `std::process::Command` + `Stdio::piped()`," and the Anthropic Agent SDK provides no help here either ([open issue #213](https://github.com/anthropics/claude-agent-sdk-typescript/issues/213)).

---

## 1. Timeline

| Day | What happened |
|---|---|
| 2026-05-11 morning | TOOL_OUTPUT_STREAMING analysis doc written. Conclusion: option B-hard (MCP bash + PreToolUse deny). |
| 2026-05-11 mid-day | SPEC_STREAMING_BASH_RUNNER detailed: tool_chunk wire format, WPS subject naming, head/tail truncation. |
| 2026-05-11 PRs landed | #800 (reducer + types) → #803 (overlay UI) → #804 (β.A: `agentmux-bashwrap` crate). |
| 2026-05-11 evening | β.B (#809-equivalent) wired bashwrap into agentmux-srv: HTTP publish endpoint, env injection, hook auto-injection, frontend WPS subscription. Build 0.33.804 packaged for smoke test. |
| 2026-05-11 late | Smoke test fails — overlay shows time hover but no streaming text. Diagnosis: sidecar log has zero `bashwrap` / `PreToolUse` mentions. Root cause: hook auto-injected at `.claude/hooks.json` which Claude Code does not read. |
| 2026-05-11 night | PR #813 force-pushed to write `.claude/settings.json` under `"hooks"` key, with user-PreToolUse prepended. Reagent approved on round 2; codex re-review pending. Smoke build 0.33.808 ships. |
| 2026-05-11 night (this retro) | User runs 0.33.808 — `.claude/settings.json` IS written now, hook IS firing, **but every bash command fails**. Sidecar log shows `task_started → task_notification status=failed` within ms. Standalone wrapper test reveals child exits with `STATUS_DLL_INIT_FAILED` *before* producing any output. Hook discovery is fixed; spawn is broken. |

---

## 2. The first solution — what was designed and why

The analysis doc landed on **Option B-hard**:

> Extend the existing `agentmux-mcp` server with a `bash` tool whose execution path: (1) generates / receives a `tool_use_id`, (2) spawns a PTY-backed `bash -c "$command"`, (3) streams stdout+stderr to a new WPS subject keyed by `tool_use_id`, (4) buffers full output, (5) returns buffered output to Claude as the MCP `tool_result`. To guarantee Claude routes through it, add a `PreToolUse` hook on native `Bash` that returns `permissionDecision: "deny"` with reason "Use `mcp__agentmux__bash` instead."

Why this and not the alternatives:

- **Option A (post-hoc synthesis from `tool_result`)** — rejected as an anti-feature: rendering a 3-minute command's output all at once at the end of the run feels worse than the existing spinner because the user is conditioned to expect live streams. We did briefly consider shipping A as a stepping stone for the overlay UI; in the end the overlay was small enough to ship without it.
- **Option C (replace Claude CLI with Agent SDK)** — 4-6 week scope, touches auth, session resume, slash commands, partial-message handling, and breaks Codex/Gemini parity since they have no equivalent SDK. Out of scope for live-log work.

**Implicit assumptions baked into B-hard that turned out to be wrong:**

1. **"`agentmux-mcp` server already exists."** It doesn't — and never did. The analysis doc said *"AgentMux already injects an MCP server (`agentmux-srv/src/backend/agent_config.rs:230-293`)"* and we read that as "the server's a binary we can extend." It's not — it's just a `.mcp.json` config entry that points at a binary we never actually built. (Memory record: `reference_agentmux_mcp_nonexistent.md`.)
2. **"`PreToolUse` can do deterministic routing via `permissionDecision: deny` with a redirect-reason."** Claude Code's hook contract does support deny + reason, but routing the model from "native Bash denied" → "mcp__agentmux__bash retry" depends on the model interpreting the reason text. It's not a hard guarantee — and given assumption #1 collapsed, we couldn't even test it.
3. **"`.claude/hooks.json` is where project hooks live."** Wrong — `.claude/settings.json` under a `"hooks"` key is the real path. The analysis doc actually had this right in §5 and §7 ("`.claude/settings.json` (under `\"hooks\"` key — the real Claude Code discovery location)"), but the code in `agent_config.rs` predated the spec and was writing `.claude/hooks.json`. The spec text and the code never reconciled.
4. **"PTY is the right transport because spinners."** True for `npm install` aesthetics, false for the actual live-log use case. The feature is *"see the log lines as they happen"* not *"see the spinner spin."* PTY is a hedge against a problem we don't have, and it locks us into platform-specific PTY shenanigans (ConPTY on Windows, openpty on macOS/Linux).

---

## 3. What we actually shipped — the deviation

Because `agentmux-mcp` didn't exist, β.A took a different shape: a standalone binary `agentmux-bashwrap` with two subcommands:

- `agentmux-bashwrap hook` — reads a PreToolUse JSON event on stdin, emits `updatedInput.command` that rewrites the bash command to invoke `agentmux-bashwrap exec` with the original command base64-encoded into argv.
- `agentmux-bashwrap exec --tool-id=… --b64-cmd=…` — spawns `cmd /C <decoded>` (Windows) or `bash -c <decoded>` (Unix) via `portable_pty`, streams output to WPS via HTTP POST, and prints the aggregated output on stdout for Claude to capture as the `tool_result`.

This is a **PreToolUse command rewrite** approach, *not* the MCP-tool-with-deny-hook approach the spec called for. The two are similar in effect but very different in failure modes:

| | MCP tool + deny hook (spec) | Command-rewrite hook (shipped) |
|---|---|---|
| **Tool surface to Claude** | New tool `mcp__agentmux__bash` | Same `Bash` tool, rewritten command |
| **Permission flow** | Native Bash denied, MCP tool may need its own allow | Same as native Bash |
| **Routing reliability** | Depends on model picking up "use mcp__agentmux__bash instead" reason | Deterministic — the hook rewrites unconditionally |
| **Hook lives at** | `.claude/settings.json` `"hooks"` key | `.claude/settings.json` `"hooks"` key |
| **Wrapper invocation** | Via MCP stdio JSON-RPC | Via shell `agentmux-bashwrap exec …` |
| **Output → Claude** | MCP `tool_result` (text content) | Wrapper prints to stdout, Claude reads as Bash `tool_result` |
| **Wrapper crash mode** | MCP server stops responding → Claude shows error | Shell command exits non-zero → Claude shows `task_notification status=failed` |
| **Provider parity** | Generalizes to Codex/Gemini if they support MCP | Generalizes to any provider with a "rewrite command" hook |

The shipped form is **more deterministic** than the spec form (no model-reasoning dependency for routing) but **more fragile** at the shell layer — every command goes through a tiny binary whose own correctness is now load-bearing for *all* bash invocations the agent makes. Which brings us to:

---

## 4. The three failure modes

### 4.1 Wrong hook discovery path (`hooks.json` vs `settings.json`)

**Symptom:** sidecar log at 0.33.804 has zero mentions of `bashwrap`, `PreToolUse`, or `tool_chunk` even though tool_use events fire normally.

**Diagnosis:** `agent_config.rs:223` (legacy path) wrote `.claude/hooks.json` from a `content_map["hooks"]` field that no agent had ever populated, so the wrong-file bug was dormant until β.B started writing the auto-injected PreToolUse there.

**Fix (PR #813):** rename `build_hooks_config` → `build_settings_with_hooks`, write `.claude/settings.json` with the auto-injected `PreToolUse:Bash` entry under the top-level `"hooks"` key. User-supplied settings.json `PreToolUse` entries are *prepended* (round 2 of the fix, after codex P2) so they short-circuit ours.

**Why we missed it in design:** the analysis doc had the right answer (§5 + §7 both mention `.claude/settings.json`), but the existing legacy code wrote `.claude/hooks.json`, and during implementation we extended the existing code path without re-reading the spec. **The spec was right, the code was wrong, and the reconciliation never happened.**

Memory record: `reference_claude_code_hooks_location.md`.

### 4.2 Wrapper child crashes with STATUS_DLL_INIT_FAILED

**Symptom:** at 0.33.808, hook fires correctly — sidecar log shows `agentmux-bashwrap exec --tool-id=… --b64-cmd=…` for every bash call — but `task_started` is immediately followed by `task_notification status=failed`. Standalone reproduction:

```
$ agentmux-bashwrap exec --tool-id=t --b64-cmd=ZWNobyBoZWxsbw
<exited -1073741502 in 0.07s>
[bashwrap] warning: streaming disabled (auth/url env missing); command output will only appear on completion
```

`-1073741502` = `0xC0000142` = `STATUS_DLL_INIT_FAILED`. The wrapper itself ran (otherwise no `<exited …>` line), but the PTY child (`cmd.exe`) died 70ms after launch without producing any output, and the wrapper faithfully reported the exit code back.

**Root cause (identified via research — wezterm discussion [#4674](https://github.com/wezterm/wezterm/discussions/4674)):** the wrapper drops `pair.master` immediately after cloning the reader:

```rust
// agentmux-bashwrap/src/bash_wrap.rs:253-257
let mut reader = pair.master.try_clone_reader()?;
drop(pair.slave);   // ← OK
drop(pair.master);  // ← THIS is the anti-pattern
```

Per the wezterm maintainer (who maintains `portable-pty`), **ConPTY does not tolerate handle closure during child startup.** Dropping `pair.master` before the child has finished initializing causes the spawned process to fail with `STATUS_DLL_INIT_FAILED` — exactly our symptom. The comment in the wrapper code (*"Drop the master writer; we never inject stdin (interactive prompts deferred to PR γ+ per spec §14 open questions)"*) reflects a misunderstanding: on Windows, the master handle isn't just a writer to stdin — it's the *pseudoconsole anchor* for the child, and dropping it tears down ConPTY mid-startup.

Why `agentmux-srv/.../shell.rs:450` works with the same `CommandBuilder` pattern: it stores `pair.master` in a long-lived task (line 788: `let master = pair.master; tokio::spawn(async move { ... });`) so the master outlives the child. The wrapper code took a shortcut for the "no stdin injection" case and broke ConPTY in the process.

**Two fix paths:**

1. **Minimal-diff fix: keep `pair.master` alive.** Don't drop it until after `child.wait()` returns. ~5 lines changed. Preserves PTY semantics (spinners, ANSI redraws).
2. **Larger refactor: drop `portable_pty` entirely, use `tokio::process::Command` + `Stdio::piped()`.** Loses PTY semantics but is simpler code, fewer Windows footguns, and cross-platform consistent.

Given the live-log feature wants *line streaming* (not spinner fidelity), and the codebase already has another consumer of `portable_pty` working correctly, **path 2 is the recommended choice for v1 of the wrapper.** PTY can come back as a follow-up if/when we actually want progress-bar fidelity, and at that point we'll know exactly which anti-pattern to avoid.

**New theory (the fix in flight):** stop using `portable_pty` + `cmd /C` entirely. The live-log feature wants line-streaming, not PTY fidelity. Switch to `tokio::process::Command` with `Stdio::piped()` running `bash -c <command>`. Three reasons:

1. **Sidesteps ConPTY init.** Plain pipe spawning on Windows uses the same CreateProcessW path Node.js, Go, Rust std, and every other language uses successfully. No pseudoconsole attach, no STATUS_DLL_INIT_FAILED risk.
2. **Bash, not cmd.** Claude sends bash syntax (`2>&1`, `[[ -f x ]]`, `pipefail`). `cmd /C` of a bash command works for the simple cases by accident and breaks for the real cases silently. Using `bash` matches what Claude's native Bash tool does internally.
3. **Cross-platform parity.** Same code path on Win + Linux + macOS. Locate bash via `$BASH` env → `which bash` PATH search → well-known Windows paths (`C:\Program Files\Git\bin\bash.exe`, etc.).

The tradeoff we lose: `npm install`'s `[==>] 50%` progress bar won't animate live, because `npm` checks `isatty(stdout)` and emits flat text when piped. For the *log* feature that's fine — the spinner/progress aesthetic is nice-to-have, line-streaming is the actual ask.

### 4.3 Env propagation drops `AGENTMUX_AUTH_KEY` / `AGENTMUX_LOCAL_URL`

**Symptom:** every wrapper invocation prints `[bashwrap] warning: streaming disabled (auth/url env missing)` — even though `agentmux-srv/src/server/websocket.rs:857` explicitly injects `AGENTMUX_AUTH_KEY` into `env_vars` for the Claude spawn, and `agentmux-srv/src/main.rs:498` sets `AGENTMUX_LOCAL_URL` in the parent process env (which should inherit).

**Chain:** agentmux-srv → portable_pty spawn of `claude` → claude reads env → claude spawns `bash` (for tool) inheriting env → bash spawns `agentmux-bashwrap hook` (via PreToolUse) inheriting env → bash spawns `agentmux-bashwrap exec` (after hook rewrites command) inheriting env. Five hops.

**Status:** unverified — we never instrumented the wrapper to log which env vars it actually receives. Right now we can't tell which of the five hops is dropping the vars. Adding `tracing::info!` in `WpsClient::from_env` to log the keys it sees is a one-line fix that should land in the same PR as the spawn rewrite, so we get diagnostic visibility on the next smoke test.

---

## 5. The new theory — pipe-based, bash-based, observable

```
                                  hook subcommand
                                  (unchanged: rewrite cmd → b64)
                                            │
                                            ▼
                                  agentmux-bashwrap exec --tool-id=… --b64-cmd=…
                                            │
                                            ▼
                                  tokio::process::Command::new(locate_bash())
                                       .arg("-c").arg(decoded_command)
                                       .stdin(Stdio::null())
                                       .stdout(Stdio::piped())
                                       .stderr(Stdio::piped())
                                       .spawn()
                                            │
                          ┌─────────────────┼──────────────────┐
                          ▼                 ▼                  ▼
                  read stdout lines   read stderr lines    wait for exit
                          │                 │                  │
                          └──────┬──────────┘                  │
                                 ▼                             ▼
                         publish to WPS                   forward exit code
                         (HTTP POST chunk:<id>)           to wrapper exit
                                 │
                                 ▼
                         frontend WPS sub
                         → dispatchDoc(ToolChunkAppend)
```

Key properties:

- **No PTY.** No ConPTY init, no STATUS_DLL_INIT_FAILED.
- **Bash on Windows.** Found at runtime via `$BASH`, `$AGENTMUX_BASH`, `which bash`, then fallback to `C:\Program Files\Git\bin\bash.exe`. Fail-loud (not pass-through) if not found.
- **Diagnostic env logging.** First thing the `exec` subcommand does is `tracing::info!(target: "bashwrap", env_keys = ?relevant_env_keys())` so the sidecar log tells us if `AGENTMUX_AUTH_KEY` / `AGENTMUX_LOCAL_URL` arrived.
- **Single dependency drop.** `portable_pty` removed from `agentmux-bashwrap/Cargo.toml`. The crate stays small.
- **Same public contract.** Hook subcommand unchanged. `exec` argv unchanged. WPS publish payload unchanged. No frontend changes.

---

## 6. Online research findings

### 6.1 Claude Code hooks — the canonical contract

- **File on disk:** project hooks live at `.claude/settings.json` (committable). User-level at `~/.claude/settings.json`. Gitignored overrides at `.claude/settings.local.json`. `.claude/hooks.json` is **not** a discovery location.
- **URL gotcha:** `docs.claude.com/en/docs/claude-code/hooks` now 301-redirects to `code.claude.com/docs/en/hooks` (verified 2026-05-11). Same content; the old URL just isn't canonical.
- **Two `hooks` keys.** The wire shape is `{ "hooks": { "PreToolUse": [{ "matcher": "Bash", "hooks": [{ "type": "command", "command": "..." }] }] } }`. Outer object key + inner array key — frequent config-bug source. Our `build_settings_with_hooks` emits the correct nesting.
- **`permissionDecision` enum:** `"allow" | "deny" | "ask" | "defer"`. Our hook emits `"allow"` with `updatedInput.command` set, which is the documented pattern for command rewriting.
- **Hook stdin/stdout contract:**
  - stdin: `{session_id, transcript_path, cwd, hook_event_name: "PreToolUse", tool_name, tool_input: {command, description, timeout, run_in_background}, tool_use_id}`
  - stdout (on exit 0): `{"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "...", "permissionDecisionReason": "...", "updatedInput": {"command": "..."}, "additionalContext": "..."}}`
  - exit codes: `0` = parse stdout JSON; `2` = blocking error (stderr surfaced to Claude); other non-zero = non-blocking (stderr to transcript).
- **Chained-command matcher caveat:** the `if` Bash matcher strips leading `VAR=value` and matches each subcommand of `&&`/`||`/`;` chains independently. Our matcher `"^(Bash|.*[Bb]ash.*)$"` doesn't use the `if` form — it's a plain `matcher` regex on `tool_name` — so this doesn't affect us, but it's worth knowing if we ever want to selectively wrap (e.g. only `npm` / `gh` commands).

### 6.2 Agent SDK doesn't help (yet)

- `includePartialMessages: true` yields `stream_event` envelopes around the raw Anthropic API events: `message_start`, `content_block_start`, `content_block_delta` (text or `input_json_delta`), `content_block_stop`, `message_delta`, `message_stop`. Same shape as `claude --output-format stream-json` — the SDK is a thin wrapper.
- **The gap:** streaming covers the model's response generation only. Once `tool_use` finishes, execution happens silently; Claude only emits the next `message_start` after `tool_result` is appended. **No `tool_execution_output` event exists.**
- Open feature request: [anthropics/claude-agent-sdk-typescript#213](https://github.com/anthropics/claude-agent-sdk-typescript/issues/213) (opened 2026-03-05, still open, no Anthropic comment as of 2026-05-11) requests exactly `tool_execution_start/output/end` events. **Not implemented.**
- `canUseTool` callback intercepts before execution (permission decision + `updatedInput`) but gives no handle to the running process. No documented in-progress subscription.

**Implication for AgentMux:** the spec-doc conclusion ("don't wait on `tool_output_delta`") still holds. Host-side interception via hook + wrapper remains the right path until/unless Anthropic ships #213.

### 6.3 `portable-pty` Windows ConPTY — known root cause

Per [wezterm discussion #4674](https://github.com/wezterm/wezterm/discussions/4674) (wez, the maintainer) and [issue #4206](https://github.com/wezterm/wezterm/issues/4206):

- **Primary root cause of `0xC0000142`:** dropping the `PtyPair` / `master` / reader / writer too early. ConPTY cannot tolerate handle closure during child startup — the child returns immediately with `STATUS_DLL_INIT_FAILED`.
- **Canonical anti-pattern:** returning `Box<dyn Child + Send + Sync>` from a function while `pair` falls out of scope in the caller. Same shape as our wrapper's eager `drop(pair.master)`.
- **Fix recipe (per wez):**
  1. Keep `pair.master` alive in the same scope as the I/O copy loop. Don't drop until after `child.wait()` returns.
  2. Run `child.wait()` on a dedicated thread, separate from the read/write loops.
  3. Don't `env_clear()` without seeding `SystemRoot` and `PATH` — `cmd.exe` needs these DLLs to init.
  4. Make sure `cwd` points at a real existing directory.
  5. **If you only need stream capture (no TTY semantics), use plain `std::process::Command` + `Stdio::piped()`.** Far fewer Windows footguns.

The match between this list and our wrapper's bug list is near-perfect — we hit #1 head-on. Path 5 (plain pipes) is what the new theory adopts; it sidesteps the entire class of issues.

### 6.4 Survey of approaches to wrap a sub-agent's bash

| Approach | Fidelity | Robustness | Complexity |
|---|---|---|---|
| **(a) PreToolUse hook command rewriting** *(our approach)* | High — raw bytes, ANSI preserved, real-time | Medium — depends on `updatedInput` semantics holding stable; shell-quoting edge cases; chained commands need care | Low–Medium — config + tiny wrapper binary |
| **(b) MCP server providing custom Bash tool** | High — full control + can stream via MCP notifications | High — official extension point; survives SDK upgrades | Medium-High — reimplement Bash semantics (cwd persistence, timeouts, background, env); must denylist built-in Bash via `disallowedTools` |
| **(c) Agent SDK + intercept `tool_use_id` chunks** | Low for execution — input deltas only, no execution output (issue #213). Still needs (a) or (b) underneath. | High at the SDK layer | High and **doesn't solve the problem** until #213 lands |

**Verdict for an IDE wrapper today:** (a) is the right pragmatic choice — minimal surface area, fidelity to actual stdout/stderr bytes, no need to reimplement Bash semantics. (b) becomes attractive if/when we want sandboxing/policy/multi-tenant isolation (since the MCP server then owns the process tree). (c) is a non-starter on its own.

### 6.5 Sources

- [Claude Code Hooks](https://code.claude.com/docs/en/hooks) — canonical; docs.claude.com redirects here.
- [Agent SDK streaming output](https://code.claude.com/docs/en/agent-sdk/streaming-output)
- [claude-agent-sdk-typescript#213](https://github.com/anthropics/claude-agent-sdk-typescript/issues/213) — open feature request for tool-execution streaming events
- [wezterm discussion #4674](https://github.com/wezterm/wezterm/discussions/4674) — `STATUS_DLL_INIT_FAILED` root cause analysis from the portable-pty maintainer
- [wezterm issue #4206](https://github.com/wezterm/wezterm/issues/4206) — portable-pty Windows write failures
- [portable-pty `CommandBuilder` rustdoc](https://docs.rs/portable-pty/latest/portable_pty/cmdbuilder/struct.CommandBuilder.html)
- [Bringing Claude Code Sub-agents to Any MCP-Compatible Tool](https://dev.to/shinpr/bringing-claude-codes-sub-agents-to-any-mcp-compatible-tool-1hb9) — survey post
- [MCP — Claude API Docs](https://platform.claude.com/docs/en/agent-sdk/mcp)

---

## 7. Lessons

1. **When the spec and the existing code disagree, reconcile in a separate commit before building on top.** The `.claude/hooks.json` path was wrong in the legacy code; the spec said `.claude/settings.json`. We extended the legacy path. **One sentence of "wait, the spec says the other file — let me fix the legacy first" would have saved a smoke build.**

2. **Don't assume a binary exists from a config-file reference.** `agentmux-mcp` appeared in `.mcp.json` injection logic; we assumed it was a real binary we could extend. It wasn't. **Grep for the binary name in the build system before scoping work that depends on it.**

3. **Reuse working code instead of reinventing.** `agentmux-srv/.../shell.rs` spawns processes via `portable_pty` + ConPTY successfully every day. The wrapper's spawn code was written from scratch and ran into a problem the working code had already solved. **If a pattern works elsewhere in the codebase, start from a copy of it.**

4. **PTY is a hedge, not a default.** "We might want spinner fidelity later" cost us a wedged feature. The right move was plain pipes + `bash -c` for v1, PTY in a follow-up if/when we actually want progress bars.

5. **Instrument before you ship the feature.** The "env vars not arriving" bug would have been one log line away from diagnosis if the wrapper logged its env on every invocation. We're adding that in the rewrite — should have been there in β.A.

6. **Smoke test the seam, not just the unit.** β.A had unit tests for hook JSON parsing, base64 round-tripping, and head/tail truncation. None of them spawned a real child. The first time `cmd /C echo` ran inside the wrapper was on the user's machine.

7. **Read the platform-specific lifetime contracts before writing platform code.** The `drop(pair.master)` that broke us was added with a comment explaining *why* we were dropping it (no stdin injection needed). That's a great answer to the Unix question (master holds the writer; drop frees it) and the wrong answer to the Windows question (master is the pseudoconsole anchor; dropping it kills the child). A 30-second skim of the portable-pty docs or the wezterm issue tracker would have caught this. **When using a cross-platform abstraction, the Windows lifetime contract is usually the strictest — start there.**

---

## 8. Forward plan

1. **(in flight)** PR #813 — `.claude/settings.json` hook discovery fix. Reagent approved; codex re-review pending after force-push to f311f0f0. Merge once codex green.
2. **(next)** PR γ — wrapper rewrite. Drop `portable_pty`, use `tokio::process` with piped stdout/stderr, run via `bash -c`. Add env-trace logging. Bump 0.33.810. Smoke test inside Claude pane.
3. **(after γ smoke)** Investigate env propagation: if `bashwrap` logs show `AGENTMUX_AUTH_KEY` / `AGENTMUX_LOCAL_URL` missing, walk the spawn chain from agentmux-srv outward to find which hop drops them.
4. **(post-streaming)** Decide whether PTY is worth a follow-up — the plain-pipes path may be good enough that the spinner-fidelity work never lands.

---

## 9. Cross-references

- Original analysis: [`docs/analysis/TOOL_OUTPUT_STREAMING_2026_05_11.md`](../analysis/TOOL_OUTPUT_STREAMING_2026_05_11.md)
- Detailed spec: [`docs/specs/SPEC_STREAMING_BASH_RUNNER_2026_05_11.md`](../specs/SPEC_STREAMING_BASH_RUNNER_2026_05_11.md)
- Live-log data shape: [`docs/specs/SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md`](../specs/SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md)
- Wrapper source: `agentmux-bashwrap/src/{main,hook,bash_wrap,wps_client}.rs`
- Hook auto-injection: `agentmux-srv/src/backend/agent_config.rs:111-128` (post-PR #813)
- Env injection at Claude spawn: `agentmux-srv/src/server/websocket.rs:843-867`
- WPS publish endpoint: `agentmux-srv/src/server/mod.rs` `handle_wps_publish`
- Frontend WPS subscription: `frontend/app/view/agent/useAgentStream.ts` chunkSubs map
