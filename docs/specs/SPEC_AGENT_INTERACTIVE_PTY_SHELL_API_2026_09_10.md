# SPEC: Agent-Driven Interactive PTY Shell API

**Author:** Agent3
**Created:** 2026-09-10
**Status:** Implemented in two passes. Pass 1 (PtyShell/PtyShellInput/
PtyShellResize/PtyShellRead/PtyShellStatus/PtyShellStop — `PtyShellSignal`
also shipped in pass 1 but was removed the same day after Codex review, §6b)
shipped
each tool driving its OWN independent, invisible shell — real-PTY round trip
(create → type a command → read its actual output → stop) verified live, not
just compiled; see §9. Pass 2 (this revision, repo owner's explicit follow-up
request) changed the model entirely: `PtyShell` now attaches to the SAME
shell a human's composer-drawer session already uses (or will use), with a
short, self-expiring lock keeping the two from typing over each other. See
§10 for the full pass-2 design and what it changed from pass 1.
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
| `PtyShellInput(shell_id, text)` | `blockcontroller::send_input(block_id, BlockInputUnion::data(text.into_bytes()), None)` — also the correct way to send Ctrl+C (`"\x03"`; see §6b, `PtyShellSignal` was removed) |
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

## 6b. Four real bugs caught by Codex review (PR #3177), not by compiling or the mocked test suite

**1. `PtyShellSignal` did the opposite of what it advertised — removed.**
The tool's own description claimed sending `SIGINT` would interrupt the
foreground process "the same mechanism a terminal's Ctrl+C keystroke
uses," implying the shell stays alive. Actual behavior, verified directly
in `blockcontroller/shell/lifecycle.rs`'s input loop: `if
input.sig_name.is_some() { break; }` — ANY signal name closes the PTY
(drops writer + master), terminating the *entire* shell, not just the
foreground command. `ShellController::stop()` (the real termination path,
used by `PtyShellStop`) is a completely separate mechanism (SIGTERM/SIGKILL
to the process group) that never goes through this code at all. Removed
`PtyShellSignal` outright rather than trying to half-fix it: the correct,
standard way to deliver a real interrupt through a PTY is the raw control
byte (`"\x03"`) via `PtyShellInput` — the OS PTY line discipline (Unix) /
ConPTY's translation layer (Windows) converts that into a genuine SIGINT to
the foreground process group without touching the PTY itself, which is
exactly what every real terminal (including xterm.js) already does and
needs no new code here.

**2. No check that `shell_id` was ever created by this API.** `shell_id`
is a real block id, in the same namespace as every other pane, and
`Layout` exposes pane block ids. Without a check, `PtyShellInput`/
`PtyShellStop`/etc. could be pointed at an arbitrary controller-backed
block — another agent's own CLI pane, a human's terminal pane — and
inject input into it or tear it down. Fixed: every block `PtyShell`
creates is stamped with a `ptyshell:managed` meta marker; every other
handler (`input`/`resize`/`read`/`status`/`stop`) checks it first and
treats a block that isn't ptyshell-managed exactly like an unknown id
(same response shape, so there's no oracle for "this id exists but isn't
mine"). Scoped to "was this created by this API," matching the existing
`Shell`/`ShellStop` family's own scope — not full cross-agent ownership
isolation (agent A can still touch agent B's ptyshell if it discovers the
id), which the pipe-based family doesn't have either.

**3. `cwd` silently fell back to `agentmux-srv`'s own directory.** Unlike
`handle_shell_create` (the existing `Shell` tool's handler), the new
`create` handler didn't fall back to the agent block's own `cmd:cwd` when
the caller omitted `cwd` — meaning the common case (no explicit `cwd`)
spawned the shell in the portable runtime directory, not the agent's
worktree. Fixed to mirror `handle_shell_create`'s exact fallback.

**4. A failed spawn left a stale, unreachable block behind.** If
`resync_controller` failed after the block was already inserted and linked
into the parent's `subblockids`, the handler returned a 500 without ever
handing back a `shell_id` — so the caller had no way to clean up the
now-orphaned block/controller registration. Fixed: roll back (delete the
controller, delete the block, unlink from the parent) on that error path.

All four are covered by new deterministic tests in
`agentmux-srv/src/server/tests.rs`
(`ptyshell_rejects_operating_on_a_block_it_did_not_create`,
`ptyshell_create_defaults_cwd_from_the_agent_block`), re-verified against
the live PTY round trip (§9) to confirm the fixes didn't regress the actual
working case.

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

**A fifth real bug, this time caught by CI itself rather than review or a
local run:** two of the deterministic tests (`create`'s own wiring test and
the cwd-fallback test) call `/api/v1/ptyshell/create` for real — which
spawns a genuine, persistent interactive shell process, not a mock — and
neither cleaned it up afterward. Locally (Windows) this was invisible: the
suite still finished in well under a second. On CI's Linux runner it hung
`cargo test --workspace` for hours (one run sat long enough to hit a ~6h
auto-cancel; a second, freshly re-triggered run was still stuck at 5h45m
when caught and manually cancelled). Root cause not fully isolated beyond
"an orphaned real shell process from a test that never stops it" — plausibly
the runner's own job-completion wait, not `cargo test`'s own process, since
the local Windows run (which does the same real spawn) exits promptly.
Fixed by having both tests call `blockcontroller::delete_controller`
directly at the end — deliberately not the HTTP `stop` endpoint, so the
cleanup can't itself be broken by a bug in that handler. This is exactly
the discipline the `#[ignore]`'d live test's own doc comment already
argued for (only intentionally-real-process tests should spawn one) — these
two just weren't meant to be in that category and had to be brought back in
line.

## 10. Pass 2 — the same shell a human sees, with a lock (repo owner request, 2026-09-10)

Pass 1 shipped `PtyShell` creating its own independent, headless sub-block —
architecturally identical to the human-facing composer-drawer shell, but a
*different block*. A human watching the drawer saw nothing the agent did; if
the human already had the drawer open, the agent's shell was invisible and
separate. The repo owner's follow-up request: the agent must drive the SAME
shell a human sees, with text appearing live as the agent works, while
staying fully API-driven (no OS-level keystroke injection, no simulated
DOM/xterm.js events) — and if a human is also typing, lock them out while
the agent is actively working, unlocking again the moment it stops.

### 10.1 Reuse, in both directions

`create` now checks the agent's own block for `term:shellsubblockid`
(`agent-view.tsx:2883,2890-2894` — the same key the composer drawer already
persists there) before doing anything else:

- **Pointer exists** → `resync_controller` against that id instead of
  creating a new block. Covers a human who already opened the drawer: the
  agent joins their exact PTY, same scrollback, same live output.
- **Nothing exists yet** → create as pass 1 did, then persist the pointer
  onto the agent block. Covers the reverse direction: a drawer opened
  *after* the agent has been working attaches to what the agent already
  started (`ControllerResyncCommand`'s own reuse path picks it up), instead
  of spawning a second, independent shell.

`req.cwd`/`rows`/`cols` are ignored on the reuse path — an already-running
process's cwd can't change post-spawn, and geometry is `PtyShellResize`'s
job, not a create-time parameter for a shell that already has a size. The
Windows ConPTY-handshake check (§6a) runs unconditionally after either
path — safe on an already-initialized shell, which simply won't have an
unanswered query sitting in its output for the poll to find.

### 10.2 Ownership reframed from "did PtyShell create it" to "is it this agent's own pane"

Pass 1's `ptyshell:managed` meta-marker check assumed every legitimate
target was a block `PtyShell` itself created — no longer true once reuse
means a HUMAN-created drawer shell is an equally legitimate target. Replaced
with `is_owned_by_agent`: fetch the target block, check its `parentoref`
equals `block:<agent_block_id>` and it's a `view:"term"` block.
`agent_block_id` is now a field on every `PtyShell*` request
(`PtyShellInputRequest` etc., `agentmux-common/src/api_types.rs`) — filled
in by `agentmux-mcp` from its own trusted env (`AGENTMUX_AGENT_BUS_ID`/
`AGENTMUX_BLOCKID`), never a model-facing tool parameter, so it can't be
forged by the calling agent's own model output. This is strictly more
correct than pass 1's check, not just adapted for reuse: it still blocks the
exact threat Codex found (an arbitrary block id discoverable via `Layout`,
e.g. another agent's own CLI pane), while also correctly covering a shell
the human created first.

### 10.3 Locking: a lease, not an explicit lock/unlock

Every successful `PtyShellInput`/`PtyShellResize` write stamps
`term:agentlockuntil` (epoch ms, `AGENT_LOCK_WINDOW_MS` = 4000ms out) on the
target block via `update_object_meta` + a `waveobj:update` broadcast — the
same two steps the `setmeta` WS handler performs, factored into
`broadcast_meta_update` since `ptyshell/*` needs the identical pattern from
a plain HTTP handler with no `WshRpcEngine` in scope. The frontend
(`AgentShellSubblock.tsx`) gates its `sendDataHandler` purely on
`agentLockedUntil() > nowTick()` — `nowTick` a signal ticking every 500ms so
the UI notices the lock lapsing even with no new server push. Read-only
`PtyShellRead`/`PtyShellStatus` never touch the lock.

**Deliberately a lease, not an explicit lock/unlock pair** — the repo
owner's own framing: "make sure to unlock as soon as the agent stops using
it... keep that binding to the shell open only when necessary." An explicit
release call is one the agent could simply never make (crash, error,
forgetting), leaving a human locked out indefinitely — the opposite of
"unlock as soon as it stops." A short, self-expiring window needs no
server-side timer to leak or get stuck: a quiet agent just lets the
timestamp lapse.

While locked, the human still **sees** live output — only their own
keystrokes are dropped, with a small corner badge ("Agent is using this
shell") explaining why, deliberately not a full-screen overlay (the entire
point of the visible-shell feature is watching the agent work).

### 10.4 `PtyShellStop` no longer kills anything

Pass 1's `stop` deleted the controller and the block — correct for that
version's model (the agent's own throwaway shell), wrong once the shell is
shared with a human's live terminal session. `PtyShellStop` now means
"release my lock immediately, don't wait for the window to lapse" — it
clears `term:agentlockuntil` and returns `released: bool`, and never touches
the process or the block. The shell lives for the pane's own lifetime now,
matching `AgentShellSubblock.tsx`'s pre-existing rule ("only killed when the
pane closes") — this tool doesn't own it outright anymore.

### 10.5 Deferred: Session/History → Stash

The repo owner separately raised, while working in this area, that the
composer details panel's `<AgentControlBar>` (session archive/restore/
export actions + "View full history") shouldn't be bundled with the shell
toggle — candidate destination is `AgentStashModal`'s existing tabbed
per-agent surface. **Not done in this pass** — scoped out as an unrelated
UI reorg rather than folded into an already-large PTY-shell change; tracked
as a separate follow-up. One open product question for whoever picks it
up: do the transient banners (interrupted-session recovery, resume-failed,
large-session warning) move into the Stash tab too, or stay inline near the
composer since they're time-sensitive?

### 10.6a Three more real bugs, caught by Codex review of pass 2 (PR #3194)

**1. The lock, as pass 2 first shipped it, didn't actually close the race
it exists for.** `AgentShellSubblock.tsx`'s gate reacts to a WS-pushed meta
value — inherently eventually-consistent, not synchronous with the
backend's own state. A human keystroke already in flight when the lock is
set could still reach `blockcontroller::send_input` and interleave with
the agent's write, exactly the collision the lease is supposed to prevent.
Fixed in the one place that's actually authoritative: `controllerinput`
(`server/websocket.rs`, the WS command every human keystroke routes
through) now checks `term:agentlockuntil` itself, at the moment it
processes the message, and drops the input rather than forwarding it. The
frontend gate stays as the UX layer (why bother sending input that'll be
dropped, and it drives the "Agent is using this shell" badge) — it's no
longer the correctness boundary.

**2. The ConPTY-handshake check, unchanged from pass 1, was newly wrong
once shells got reused.** Pass 1's version scanned the ENTIRE persisted
`term` file for `ESC[6n` — safe there, because every shell was freshly
created with an empty file. Pass 2 calls the same check on the reuse path
too, and the `term` file is append-only for a block's whole lifetime
(`handle_append_block_file`'s `FILE_OP_APPEND`) — so a long-running reused
shell's history almost always still contains its own original,
long-since-answered query from whenever it first started. Scanning the
whole file found that stale byte sequence and injected an unsolicited
`ESC[1;1R` into whatever a human was doing at that exact moment, on every
single reuse. Fixed by recording the file's length immediately before
`resync_controller` runs and only ever inspecting bytes appended after
that baseline — correct uniformly whether the shell was already running
(nothing new appears, correctly finds nothing) or had to be respawned from
dead (sees only that fresh process's own new query, never anything from
its former life).

**3. Two concurrent `PtyShell` calls on a not-yet-initialized pane could
each create their own shell.** The pointer check at the top of `create` is
a plain read, not authoritative against a second request racing it. Fixed
with an atomic claim: re-check the pointer INSIDE a `with_tx` transaction,
serialized against every other `Store::*` call via the store's single
connection lock, and only insert+link+set the pointer if it's still unset
by the time this transaction runs; the loser pivots to reusing the
winner's shell instead of proceeding with its own. **Explicitly does not
close the equivalent race against the frontend's own `createsubblock`
path** — that's a separate, non-transactional, multi-step RPC sequence
this backend call has no way to serialize against. A human opening the
drawer for the very first time on a pane in the same narrow window as an
agent's first `PtyShell` call on it can still each end up with their own
shell. Documented here as a known, accepted gap (Codex rated this P2, not
P1) rather than silently left unmentioned — closing it fully would need a
change to the frontend's own creation path too, out of scope for what one
backend transaction can enforce unilaterally.

All three reproduced the reasoning above by direct code inspection before
being accepted as real (not just taking the review's word for it) — see
§10.6's test list for what backs each one.

### 10.6b Two more real bugs, caught by ReAgent's round-2 review of PR #3194

**P1 — §10.6a #1's fix shrank the lock race but never actually closed the
part it could have.** The lock was still only written on the SUCCESS path,
after `blockcontroller::send_input` returned — meaning `send_input`'s own
entire duration (from the caller's write until it returns) ran with no
lock in place at all, so a human keystroke landing in that window could
still interleave with the agent's write via `controllerinput`, which reads
`term:agentlockuntil` before `send_input` but has nothing to see yet if
this handler hasn't written it. Fixed by moving
`lock_shell_for_agent(&state, &req.shell_id)` to run **before** the
`send_input` call in both `handle_pty_shell_input` and
`handle_pty_shell_resize`, unconditional on the write's outcome rather than
gated on success. This shrinks the remaining race to the (much smaller,
and judged proportionate to leave open — closing it fully would need a
shared mutex around the whole write, not just a meta flag) window between
`controllerinput`'s own lock check and `send_input`'s actual write for a
message that arrived concurrently with the FIRST lock-setting call itself.

**P2 — the "winner's block vanished" fallback in the atomic-claim logic
(§10.6a #3) built a phantom block instead of a real one.** When this
request loses the claim race (`Ok(Some(winner_id))`) but
`try_attach_to_existing_shell` finds the winner's block already gone (an
edge case rated "exceedingly unlikely" when §10.6a #3 was written), the
code fell back to `(child_id.clone(), block)` — reusing the in-memory
`block` value this request had prepared. But that value was only ever
`tx.insert`-ed inside the `Ok(None)` arm (the "we won the claim" path);
along the `Ok(Some(...))` arm it was never persisted at all.
`resync_controller` then ran against a block with no row behind it, and
the handler returned 200 with a `shell_id` that every subsequent
`PtyShellInput`/`Read`/`Status`/`Stop` call would fail to find via
`is_owned_by_agent`'s own lookup. Fixed by actually creating the block for
real in this fallback branch — `state.wstore.insert(&mut block)`, link it
into the parent's `subblockids`, set `META_KEY_SHELL_SUBBLOCK_ID` on the
parent, and broadcast the resulting `waveobj:update` — the same sequence
the `Ok(None)` arm's transaction performs, just run as a plain
non-transactional sequence since this code path is already past that
transaction's scope. Any store error at this point now surfaces as a
proper 500 instead of silently proceeding with an uninserted phantom.

### 10.6 Verification

All pass-1 tests updated for the new request/response shape
(`agent_block_id` added, `stopped` → `released`) and re-verified, plus three
new deterministic tests
(`ptyshell_create_reuses_the_pane_s_existing_shell_instead_of_spawning_a_second_one`,
`ptyshell_rejects_operating_on_a_block_outside_the_calling_agents_own_pane`,
`ptyshell_input_locks_the_shell_and_stop_releases_it_early`) and three new
frontend tests (`AgentShellSubblock.test.tsx`'s "agent lock" suite: drops
keystrokes while locked, forwards them once the lock has passed, badge
mounts/unmounts with the lock state). The `#[ignore]`'d live-PTY test was
extended in place to assert the lock gets set on a real successful write and
that `stop` releases it without killing the shell (`running: true`
afterward) — re-run live, still green, including through the §10.6a #2 fix
(no more spurious `ESC[1;1R` injection on a reused shell). §10.6a #1 (the
backend-side lock enforcement) is covered by a new dedicated test,
`controllerinput_is_dropped_while_an_agent_lock_is_active`
(`server/websocket.rs`) — proves a locked target is dropped before ever
reaching `send_input`, with an unlocked-target control case proving the
call path is genuinely live rather than trivially always-dropped. §10.6a #3
(the atomic claim) is exercised indirectly by the existing reuse/create
tests (all still pass with the new transactional path) but has no dedicated
concurrent-request test — writing one that reliably provokes the actual
race would need real parallel task orchestration against the same store,
judged disproportionate for a fix whose own documented residual gap (the
frontend-race half) isn't closed either way. Full `agentmux-srv` suite
(3337 tests) and the frontend typecheck re-run clean, no regressions.

§10.6b's two fixes were verified the same way: P1 by re-running
`ptyshell_input_locks_the_shell_and_stop_releases_it_early`, updated to
assert the lock is set by the real HTTP call itself (no more manual
`lock_shell_for_agent` simulation in the test) — proving the handler, not
the test, now performs the lock-before-write ordering. P2 has no dedicated
new test (reliably provoking "the winner's block was deleted between the
transaction and the attach attempt" needs orchestrating a real deletion
mid-request, judged disproportionate for a fallback branch already this
narrow) but is covered indirectly: the existing reuse/create tests and the
`#[ignore]`'d live-PTY test all still pass against the changed code path,
and the fix was traced against the exact store APIs (`insert`/`must_get`/
`update`) already exercised elsewhere in this file. Full `agentmux-srv`
suite (3337 tests, 7 ignored) re-run clean after both §10.6b fixes,
including the `#[ignore]`'d live-PTY test run explicitly — no regressions.
