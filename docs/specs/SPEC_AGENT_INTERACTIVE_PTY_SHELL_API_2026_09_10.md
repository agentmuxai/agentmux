# SPEC: Agent-Driven Interactive PTY Shell API

**Author:** Agent3
**Created:** 2026-09-10
**Status:** Implemented (PtyShell/PtyShellInput/PtyShellSignal/PtyShellResize/
PtyShellRead/PtyShellStatus/PtyShellStop) — real-PTY round trip
(create → type a command → read its actual output → stop) verified live,
not just compiled; see §9.
**Scope:** New MCP tool(s) letting an agent drive a real, PTY-backed shell —
create it, send it input (including control characters/signals), read its
current screen state, resize it, stop it — entirely through backend RPC,
with no dependency on window/pane focus or even the shell being visually
mounted anywhere.
**Related:** `docs/specs/SPEC_PERSISTENT_SHELL_NODE_2026_06_11.md` (the
existing agent-facing `Shell`/`ShellInput` tools — explicitly non-PTY, see
§1), `docs/specs/SPEC_MODULARIZE_SHELL_2026_07_02.md`,
`docs/specs/SPEC_TERMINAL_SCROLLBACK_PERSISTENCE_2026_07_23.md`,
`agentmux-srv/src/backend/shell_node.rs` (pipe-based ShellNodeRunner),
`agentmux-srv/src/backend/blockcontroller/shell/` (the real PTY
`ShellController`, already shared by the `term` widget and the agent's own
CLI subprocess), `agentmux-srv/src/backend/blockcontroller/mod.rs`
(`send_input`, `BlockInputUnion`), `agentmux-srv/src/server/app_api/blockfile.rs`
(`blockfile:read_range`/`line_count`/`read_state`),
`frontend/app/view/agent/components/AgentShellSubblock.tsx` (the "shell
under the agent pane" this spec starts from).

---

## 1. Motivating incident

While bootstrapping AgentMux's WinGet package (PR #3159, 2026-09-10), the
one-time `wingetcreate new` submission needed to answer several interactive
prompts (PackageIdentifier, Publisher, License, …). Two attempts to drive it
from an agent both failed identically:

1. Bash tool with `< /dev/null`: `Sharprompt requires an interactive environment`.
2. AgentMux's own `Shell`/`ShellInput` MCP tools (`capture_stdin: true`):
   same failure, same exit code.

Both failures are **the same root cause**, not two different bugs:
`ShellNodeRunner` (`agentmux-srv/src/backend/shell_node.rs:11-14`, its own
header comment) explicitly spawns the child with `tokio::process::Command`
and **captured pipes — "no PTY in this phase."** `ShellInput` writes text
into that pipe (`shell_node.rs:403-417`), which is real stdin data, but any
program that checks `isatty()`/console-interactivity (readline libraries,
Sharprompt, `sudo`, `ssh`, `npm login`, `git` credential prompts, PowerShell
secure-string prompts, `wingetcreate new`, and most first-run setup wizards)
detects the pipe and either refuses outright (as here) or silently
misbehaves (no ANSI rendering, wrong echo, no cursor addressing).

This was foreseen, not accidental: `SPEC_PERSISTENT_SHELL_NODE_2026_06_11.md`
§3 lists "Full interactive PTY (keyboard input from the agent pane UI)" as an
explicit **non-goal**, and §10 Q3 says outright: *"Full interactive PTY
(xterm.js rendering) is out of scope — use the `term` widget for that."* That
widget exists and is genuinely PTY-backed — but, per that same spec's
Problem statement (§1): *"agents cannot open or interact with a terminal
widget programmatically."* This spec is about closing exactly that gap.

## 2. Current state (researched 2026-09-10, not from memory)

Two separate, non-interoperating shell mechanisms exist today:

| | `ShellNode` (agent-facing) | `term`-widget PTY (human-facing) |
|---|---|---|
| Entry point | `agentmux-mcp`'s `Shell`/`ShellInput`/`ShellStatus`/`ShellStop` tools | `CreateSubBlockCommand` (frontend RPC) → `view: "term", controller: "shell"` sub-block |
| Backend | `ShellNodeRunner` (`shell_node.rs`), `tokio::process::Command`, piped stdio | `ShellController` (`blockcontroller/shell/{controller,pty,lifecycle}.rs`), `portable_pty` — a **real PTY** |
| Input path | `ShellInput` → mpsc relay → `ChildStdin` (a pipe) | `blockcontroller::send_input(block_id, BlockInputUnion, seq)` (`blockcontroller/mod.rs:373`) — raw bytes, named signals (`SIGINT` etc.), or resize, written to the PTY master |
| Output path | `shell_chunk` WPS events, line-buffered stdout/stderr | `blockfile:read_range` / `:line_count` / `:read_state` RPC (`server/app_api/blockfile.rs`) over the PTY's persisted output file, plus live WS streaming |
| Agent-callable today? | **Yes** | **No** — `send_input`/`blockfile:*` are real backend primitives, but nothing wires an MCP tool to them for an agent's own use |
| Focus-dependent? | No | **No, already** — `send_input` takes a plain `block_id`; `AgentShellSubblock.tsx`'s own comment (lines 8-14) states the PTY "is only killed when the pane closes," independent of whether its xterm.js renderer is currently mounted (drawer open/closed) |

**The important finding this spec rests on:** the hard part — a real,
resize-aware, signal-capable PTY, with output persisted well enough to
survive reconnects (`SPEC_TERMINAL_SCROLLBACK_PERSISTENCE_2026_07_23.md`) —
**already exists, is mature, and is already focus-independent** (it backs
the agent's own CLI subprocess and the `term` widget alike, `is_agent_pane()`
in `controller.rs:203`). Nothing here needs a new PTY implementation. What's
missing is purely an **agent-facing API surface** onto primitives that
already work exactly the way this spec's requirements ask for.

## 3. Goals

- An agent can call an MCP tool to open a **real PTY-backed shell**
  associated with its own pane (reusing the existing `term`/`shell`
  controller — not a new PTY implementation).
- The agent can send it arbitrary input: literal text, control characters
  (Ctrl+C etc. — already modeled as `BlockInputUnion::signal`), and resize
  events — sufficient to drive `wingetcreate new`-style wizards, `sudo`
  prompts, `ssh`, interactive installers, REPLs.
- **Must work with no UI involved at all — not "unfocused," not "minimized,"
  the frontend need not be rendering or even connected.** This is a hard
  requirement, not a nice-to-have: the facility is a pure backend
  data-channel operation between the MCP tool and `agentmux-srv`, the same
  posture `send_input`/`blockfile:read_range` already have today (§2's
  table) and the same posture the *existing* `Shell` tool already has today
  (its `POST /agentmux/shell/create`-style call goes straight to `srv`, with
  zero frontend participation — confirmed by using it throughout an entire
  session with the window unfocused). The new tool is a thin agent-facing
  wrapper reusing that same call shape, not new plumbing.
- **Explicitly not keyboard/UI-input simulation.** Input must never be
  delivered by simulating keystrokes into a rendered window, or by
  dispatching synthetic DOM/xterm.js events as if a human clicked into a
  terminal and typed (that would be `UIClick`/`UIQuery`-style automation,
  and would genuinely require a live, rendered window). Input goes straight
  to the PTY master via `blockcontroller::send_input`, on the backend,
  full stop. See §6's implementation note — this constrains which existing
  code path the block-creation step may reuse.
- The agent can read back **what the shell currently shows** well enough to
  decide what to send next. See §5 for why this is the one genuinely open
  design question, not a restated goal.
- The agent can resize the PTY (some wizards render differently or wrap
  badly at the fallback geometry).
- The agent can query running/exited status and stop the shell.
- Works whether or not the agent (or a human) ever opens the composer
  drawer / `AgentShellSubblock` UI for it — the shell must be fully
  functional file/backend-object, with the UI as an optional viewer, not a
  hard dependency.

## 4. Non-goals

- General GUI/computer-use automation (mouse, arbitrary windows) — this is
  text-console interactivity only, the same scope `ShellNode` already has.
- Replacing `ShellNode` for the common case (non-interactive commands,
  piped output) — that tool is simpler and correct for its scope; this is
  an **addition**, not a migration.
- Rendering the interactive shell as a first-class top-level pane — it
  stays a sub-block of the calling agent's own block, matching
  `AgentShellSubblock`'s existing "Model A" placement.
- Solving credential/secrets handling for what an agent might type into a
  password prompt via this facility — out of scope for this spec, but
  worth naming explicitly: whoever implements this should decide whether a
  known password-prompt pattern (`sudo`'s `[sudo] password for`, etc.)
  needs special handling, since that text will otherwise flow through the
  same read/log path as everything else.

## 5. The open design question: raw bytes vs. rendered screen state

Naively, "read the shell's current output" could mean "give me the raw byte
stream since I last read" — which is exactly what `blockfile:read_range`
already returns. That is **not sufficient** for driving a real interactive
wizard: TUI programs redraw in place (cursor moves, line overwrites,
progress bars, `\r`-only updates), so a raw byte log does not tell the agent
what is actually on screen right now — the same gap that would make a naive
transcript of `wingetcreate new`'s output hard for an agent to parse
reliably once it starts overwriting lines.

Two options, not mutually exclusive:

1. **Raw range read** (already exists, `blockfile:read_range`) — cheap,
   always available, good enough for line-oriented prompts (most of
   `wingetcreate new`'s wizard is actually this — one question per line).
2. **Rendered screen snapshot** — feed the PTY's byte stream through a
   headless terminal emulator (server-side) and expose "current screen
   contents as text," the same information xterm.js already computes
   client-side for human viewing. This is the only reliable way to handle
   full-screen TUIs (progress bars, `sudo`'s prompt redraws, anything using
   cursor-addressing). Needs research into whether a suitable
   headless-terminal crate exists in the Rust ecosystem (equivalent to
   `node-pty` + `xterm-headless` on the Node side) or whether the existing
   frontend xterm.js buffer could be queried instead (that would reintroduce
   a UI dependency this spec explicitly wants to avoid — see §3's focus
   requirement — so a server-side emulator is preferred if one exists).

**Recommendation:** ship raw range reads first (near-zero new work, unlocks
the line-oriented majority of interactive-wizard cases like the motivating
incident), and treat the rendered-snapshot path as a fast-follow gated on
finding/validating a headless terminal-emulation crate. Do not block the
whole facility on solving the harder full-screen-TUI case.

## 6. Proposed architecture

New MCP tool(s) — naming TBD, sketched here as `PtyShell*` to avoid
colliding with the existing `Shell*` family:

| Tool | Backend call it wraps |
|---|---|
| `PtyShell(cwd?, title?, size?)` → `shell_id`/`block_id` | `CreateSubBlockCommand`-equivalent: create a `view: "term", controller: "shell"` sub-block parented to the calling agent's own `block_id` (same shape `AgentShellSubblock.tsx:270-278` already constructs, just invoked from a backend RPC handler instead of the frontend) |
| `PtyShellInput(shell_id, text)` | `blockcontroller::send_input(block_id, BlockInputUnion::data(text.into_bytes()), None)` |
| `PtyShellSignal(shell_id, name)` | `blockcontroller::send_input(block_id, BlockInputUnion::signal(name), None)` |
| `PtyShellResize(shell_id, rows, cols)` | `blockcontroller::send_input(block_id, BlockInputUnion::resize(...), None)` |
| `PtyShellRead(shell_id, since?)` | `blockfile:read_range` (raw) now; rendered-snapshot variant per §5 as a fast-follow |
| `PtyShellStatus(shell_id)` | Same shape as the existing `ShellStatus` tool — running/exited/exit_code |
| `PtyShellStop(shell_id)` | Existing block-close/kill path (same one the pane-close cleanup in `agent-view.tsx` already calls — see `AgentShellSubblock.tsx`'s own doc comment) |

**As implemented:** the handlers live directly in `agentmux-srv/src/server/mod.rs`
(`handle_pty_shell_*`, next to the existing `handle_shell_*` family), not a
separate module — that's where `handle_shell_create` et al. already live,
and splitting them out wasn't warranted for seven thin handlers. `PtyShellRead`
ended up reading the block's `term` FileStore file directly (whole-file read
+ in-memory tail) rather than reusing `blockfile:read_range`'s optimized
index path — that path solves problems (byte-offset seeking on huge files,
cross-channel global-transcript fallback) a freshly-created, short-lived PTY
shell doesn't have, and reusing it as-is would've meant either exposing it
outside the per-connection `WshRpcEngine` or duplicating its non-trivial
internals for no real benefit. `agentmux-mcp`'s tool definitions
(`tool_schemas.rs` + `main.rs`) follow the existing `Shell`/`ShellInput`
pattern exactly, as planned.

**Reuse, don't fork:** this explicitly reuses the `term`/`shell` controller
that already backs the agent's own CLI subprocess and `AgentShellSubblock`
— it does not create a second PTY implementation alongside
`ShellNodeRunner`'s pipes. The two tool families (`Shell*` pipe-based,
`PtyShell*` PTY-based) coexist deliberately: most agent shell usage (build
commands, watchers, one-shot scripts) has no need for PTY overhead and is
well served by the simpler, already-shipped `ShellNode`.

## 6a. Real finding: Windows ConPTY blocks forever without a terminal emulator answering it

Discovered by an actual live round-trip test, not predicted in the original
draft of this spec: the very first `PtyShellRead` after creating a shell on
Windows showed the PTY's entire output was a single `ESC[6n` (a Device
Status Report / cursor-position query) — and it never advanced past that,
for over 10 seconds, regardless of what was typed into it. Root cause:
creating a Windows ConPTY (`portable_pty`'s Windows backend, via
`CreatePseudoConsole`) makes it query the cursor position from whatever is
on the other end of the pipe, and block its own internal state (and
therefore all further output) until it gets a `ESC[<row>;<col>R` reply. A
human-facing `term` pane gets this for free — xterm.js, running in the
browser, is a real terminal emulator and answers real cursor queries as
part of its normal job. This facility has nothing playing that role: it's a
raw byte capture, not a terminal emulator, so the PTY sat permanently
stuck before printing even its first prompt.

Switching the shell from pwsh to `cmd.exe` (§6's implementation, originally
motivated by a *different*, plausible-but-wrong theory — that PSReadLine's
own cursor tracking was the culprit, per
`SPEC_AGENT_SHELL_PSREADLINE_THAW_VISIBLE_RESIZE_2026-08-14.md`) did **not**
fix this — the hang was identical under cmd.exe, proving it's ConPTY itself
initializing, not a PSReadLine-specific behavior. **Fix:** `PtyShell`'s
create handler now polls the newly-created shell's output for up to 1
second immediately after spawn; the moment it sees `ESC[6n`, it answers with
a synthetic `ESC[1;1R` via the same `send_input` path `PtyShellInput` uses,
*before* returning the `shell_id` to the caller — so by the time an agent's
tool call returns, the shell has already cleared ConPTY's handshake and is
ready for real input. `ESC[1;1R` (row 1, col 1) is a plausible-enough fixed
answer: nothing has rendered anything yet, and ConPTY only wants to
reconcile its internal buffer, not validate the value against anything a
person would see.

**This is a narrow, Windows-specific mitigation, not a general one.** It
answers exactly the one query ConPTY itself issues at creation. Any other
program that queries terminal capabilities via escape sequences and blocks
for a reply — not just at startup, but repeatedly, or for a different query
than `ESC[6n` — would hit the same class of problem, unanswered. This is the
real, concrete shape of §5's "raw bytes vs. rendered screen state" question:
a genuine headless terminal emulator (feeding the byte stream through a real
VT interpreter and answering whatever it asks, on an ongoing basis) would
subsume this one-shot fix entirely. Worth treating this as the strongest
evidence yet for prioritizing that fast-follow, not just a nice-to-have.

## 7. Security / scope note

An interactive PTY is strictly more capable than a piped shell: it can
drive `sudo`, answer credential prompts, and interact with anything a human
terminal session could. This is not a new trust boundary — it runs with the
same local privileges an agent's `Shell`/Bash tool calls already have — but
it removes one practical friction that today accidentally limits what an
agent can automate (many interactive prompts simply fail closed, as in the
motivating incident, rather than succeed). Worth a deliberate look from
whoever implements this: should `PtyShellInput` be logged/narrated with the
same visibility `ShellNode`'s output already gets (§4 of
`SPEC_PERSISTENT_SHELL_NODE_2026_06_11.md`), so a human reviewing the agent's
transcript can see what was typed into an interactive prompt, not just that
a shell ran?

## 8. Open questions for the repo owner

1. Tool naming — `PtyShell*` here is a placeholder; may want something that
   reads better alongside `Shell`/`ShellInput` (e.g. `InteractiveShell*`).
2. Is a rendered-screen-snapshot read (§5, option 2) worth the research cost
   up front, or is raw range reads (option 1) an acceptable v1 given it
   already covers the motivating incident's actual failure mode (a
   line-oriented wizard, not a full-screen TUI)?
3. Should `PtyShellStop` also be reachable from the existing `AgentShellSubblock`
   UI's stop affordance, or does the UI keep its own separate close path?
4. Logging/narration posture from §7 — default on, default off, or
   configurable?
5. §6a's ConPTY handshake mitigation is Windows-only and answers exactly one
   known query. Worth deciding now whether the general headless-terminal-
   emulator fast-follow (§5/§6a) is worth scheduling proactively, given it's
   no longer a hypothetical — it's the difference between "handles the one
   query we've seen" and "handles whatever a given interactive program
   actually asks."

## 9. Verification

Not just compiled — a real end-to-end round trip was run live
(`cargo test -p agentmux-srv --bin agentmux-srv
ptyshell_create_input_read_stop_round_trips_through_a_real_pty -- --ignored
--nocapture`, `agentmux-srv/src/server/tests.rs`): create a shell with no
window/frontend involved, type `echo <marker>`, read the PTY's real output
back, and see the actual marker in it — a genuine Windows cmd.exe session
(banner, prompt, echoed input, real output, next prompt), not a mock. Status
and stop were verified in the same run (`running: true` while live, `false`
after stop). This test is `#[ignore]`d by default, matching this codebase's
existing convention for real-process/real-daemon tests (e.g.
`backend::container`'s Docker-gated test) rather than the mocked
`ConnInterface` every other PTY test in `blockcontroller::shell::tests`
uses — real PTY spawning has genuine timing variance across machines that
mocking exists specifically to avoid in the suite that runs on every commit.

Four additional, fully deterministic tests (no real process spawn, run in
CI normally) cover the wiring: `create` inserts a real `view:"term"` block
with the right meta and links it to its parent; `input`/`status`/`stop`
against an unknown `shell_id` return the documented "not running" shape
rather than an error, matching `ShellStatus`'s existing contract.

The stop endpoint's `stopped` flag was caught wrong by the deterministic
test before it ever needed a live run: `Store::delete` succeeds (no-ops)
even for a row that never existed, so `.is_ok()` alone couldn't distinguish
"stopped a real shell" from "unknown id" — fixed to check controller
existence (`get_block_controller_status`) before tearing anything down.
