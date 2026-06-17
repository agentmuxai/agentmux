# SPEC: Tee redirected tool output to the feed + render tool output as a terminal

**Date:** 2026-06-17
**Status:** Implemented (smike) — F1 hook `tee` rewrite + F2 `TerminalOutput`. One
deviation: `TerminalOutput` lives in `view/agent/components/` (next to the cap
utilities and sibling renderers) rather than `element/`, so the view→element
dependency direction stays correct; it imports `AnsiLine` from `element/`.
**Author:** analysis pass over `main` @ `f958fdd0`
**Components:** `agentmux-bashwrap` (hook), `frontend/app/view/agent/components/*`, `frontend/app/element/ansiline.tsx`

---

## 1. Context — two complaints, one theme

Agents run shell commands and we want their real output visible in the live tool feed.
Two distinct gaps break that:

1. **Redirected output disappears.** When an agent runs `task package > build-portable.log 2>&1`,
   the shell sends stdout to the file, so AgentMux's tool feed shows **nothing** while the
   command runs. We want a facility that recognizes output is being redirected to a file and
   surfaces it to **both** the tool feed **and** the file.

2. **Task/tool output renders as a JSON blob.** Output from background-task / `Task` /
   `TaskOutput`-style tools (and any tool AgentMux doesn't special-case) renders as
   pretty-printed JSON instead of looking like a **terminal**.

Both are fixable with small, targeted changes that **reuse existing infrastructure** — no
new streaming protocol, no embedding xterm.js in the feed.

---

## 2. Background — how tool output flows today (verified)

```
Claude stream-json (tool_call: Bash, command)
  └─ PreToolUse hook  agentmux-bashwrap/src/hook.rs:53-96
        rewrites command → `agentmux-bashwrap exec --tool-id=<id> --b64-cmd=<b64(command)>`
  └─ agentmux-bashwrap exec  agentmux-bashwrap/src/bash_wrap.rs
        runs `bash -c "{ <command>; } </dev/null"` under a PTY (pipes fallback)
        reads PTY line-by-line → strips ANSI, collapses CR
        publishes each line as WPS `tool_chunk` {op:"chunk",kind,content} scoped block:<id>
        + buffers for the final model blob (`<exited N in Ts>\n…`)
  └─ frontend useAgentStream.ts subscribes tool_chunk
  └─ reducer ToolChunkAppend  agent-document/reducer.ts  → ToolNode.log.chunks
  └─ render: ToolOverlayLog.tsx → ChunkList (live) / ToolOverlayResult (final)
```

Final/structured results dispatch by tool name in `ToolOverlayLog.tsx:233-312`
(`renderToolResultBody`): `Bash`→`BashOutputViewer`, `Read`→`HighlightedCode`,
`Edit`→`DiffViewer`, `Grep`/`Glob`/`Agent`/`Task`/**default**→`CompactResult`.
`CompactResult.tsx:104,144` renders `JSON.stringify(result, null, 2)` in a `<pre>` — **this is
the "JSON blob."**

Reusable assets already in the tree:
- Live chunk pipeline (WPS → reducer → `ChunkList`) — proven by
  `frontend/test/replay/bash-with-live-log.replay.test.ts`.
- `AnsiLine` (default export, props `{ line: string }`) at `frontend/app/element/ansiline.tsx` —
  parses SGR (`\x1b[…m`) into `text-ansi-*`/`bg-ansi-*` spans.
- Output capping: `output-cap.ts` (`capText`, `MAX_TOOL_OUTPUT_LINES`).

---

## 3. Feature 1 — tee redirected output to the feed

### 3.1 Root cause
The command string is opaque to AgentMux: the hook base64-encodes it verbatim
(`hook.rs:79-85`) and bashwrap runs it as-is. A `> FILE` sends the inner shell's stdout to the
file, so the PTY (hence the chunk stream, hence the feed) sees nothing. There is **no
redirect inspection anywhere** today.

### 3.2 Design — detect a trailing file redirect at hook time and inject `tee`
Do it in the **PreToolUse hook** (`agentmux-bashwrap/src/hook.rs`), where the command is still
plaintext and already being transformed. Reasons: single insertion point, no shell-parsing in
the hot exec path, fully unit-testable (the hook already has a `build_response` test seam), and
it reuses the *entire* existing chunk pipeline downstream with **zero frontend changes** — the
teed output simply becomes normal chunks.

Transform recognized trailing redirects so stdout still flows through the PTY via `tee`, while
the file is still written:

| Original (suffix)        | Rewritten command                                  |
|--------------------------|----------------------------------------------------|
| `CMD > FILE`             | `set -o pipefail; { CMD ; } \| tee -- FILE`        |
| `CMD >> FILE`            | `set -o pipefail; { CMD ; } \| tee -a -- FILE`     |
| `CMD > FILE 2>&1`        | `set -o pipefail; { CMD ; } 2>&1 \| tee -- FILE`   |
| `CMD >> FILE 2>&1`       | `set -o pipefail; { CMD ; } 2>&1 \| tee -a -- FILE`|
| `CMD &> FILE` (bash)     | `set -o pipefail; { CMD ; } 2>&1 \| tee -- FILE`   |
| `CMD &>> FILE`           | `set -o pipefail; { CMD ; } 2>&1 \| tee -a -- FILE`|

The rewritten string is what gets base64-encoded into `--b64-cmd`. `set -o pipefail` is
**required** so the pipeline's exit code reflects `CMD`, not `tee` — bashwrap mirrors that exit
code to the model and the terminal event.

**Semantics preserved:** `tee` writes the same bytes to `FILE` that the bare `>` would have
(stdout for the no-`2>&1` forms; stdout+stderr for the `2>&1`/`&>` forms), and passes them
through to the PTY so they appear live. stderr that wasn't redirected still reaches the PTY as
before.

### 3.3 Recognition rules (conservative — bail to verbatim on anything ambiguous)
Implement a small **quote-aware** scan that only accepts a redirect that is the **top-level
trailing** element of the command. Recognize: optional `2>&1`/`&>`/`&>>`, a single `>`/`>>` to
one **regular-file** target token, with `2>&1` in the trailing position (`> F 2>&1`).

**Bail (encode the original unchanged — today's behavior) when any of:**
- the `>`/`>>` is inside single/double quotes, backticks, a `$(…)`/subshell group, a `${…}`
  parameter expansion, a comment, or a heredoc body (it's literal / not top-level),
- the target is `/dev/null` (intentional discard — don't resurrect it to the feed),
- fd-specific or dup redirects only (`2> f`, `>&2`, `3> f`), the leading-`2>&1` order
  (`2>&1 > F`, whose semantics differ), or process substitution `>(…)`,
- the redirect isn't trailing (`CMD > f | grep x`, `CMD > f && other`),
- more than one output redirect, or a top-level list operator (`&&`/`||`/`;`/`&`) **or an
  unquoted newline** is present (the redirect would bind to only one command of the list, and
  for a multi-line command wrapping `{ … }` would tee the earlier lines' stdout into the file),
- a top-level **pipe** (`a | b > f`) is present — `set -o pipefail` on the tee wrapper would
  change the inner pipeline's exit-code semantics (report the first failure instead of the last
  stage's exit) vs the original, so only single commands are rewritten,
- the command already starts with `agentmux-bashwrap exec` (idempotence, like today).

Subshell groups (`(a && b) > f`) ARE handled — the group's stdout is exactly what the trailing
`>` redirected, so wrapping it in `{ ; }` is correct (the `&&` inside the parens doesn't bind the
redirect). Backslash-newline line continuations are also fine (the escape consumes the newline).
Bailing is always safe: it yields exactly the current behavior for that call. Start narrow; widen
the grammar later if real commands need it.

### 3.4 Tests (`agentmux-bashwrap`, alongside `hook.rs` tests) — implemented
- each row of §3.2 rewrites correctly and round-trips through base64;
- subshell + quoted-target + backslash-newline-continuation forms rewrite correctly;
- `/dev/null`, quoted `>`, comment, heredoc, process-subst, `>` in/after a pipe, a multi-line
  command, `2>f`-only, `>&2`, `2>&1 > f`, double-redirect, background, and no-redirect commands
  are **passed through unchanged**;
- exit-code preservation: the rewritten form starts with `set -o pipefail`.

---

## 4. Feature 2 — render task/tool output like a terminal, not JSON

### 4.1 Root cause
`renderToolResultBody` (`ToolOverlayLog.tsx:233-312`) routes `Task` and every unrecognized tool
to `CompactResult`, whose expanded view is `JSON.stringify(result, null, 2)`
(`CompactResult.tsx:104,144`). When the result is really terminal output carried in a string
field (`content` / `output` / `stdout`), the user sees escaped-newline JSON instead of the text.
`summarize()` already *extracts* those string fields for the one-line summary
(`CompactResult.tsx:68-75`) — but the expanded body ignores that and dumps JSON.

### 4.2 Design — a shared `TerminalOutput` renderer + a "looks-like-terminal" gate
1. **New component** `TerminalOutput.tsx`:
   - props `{ text: string; class?: string; from?: "head" | "tail" }`;
   - splits on `\n`, renders each line through `AnsiLine` inside a monospace, dark, scroll-capped
     container (terminal look). Reuses `capText`/`MAX_TOOL_OUTPUT_LINES` + `OutputHiddenMarker`
     for the cap, matching `CompactResult`/`BashOutputViewer`.
   - SGR colors render via the existing `text-ansi-*` classes; non-SGR control bytes are inert
     (AnsiLine only matches `\x1b[…m`).
   - *Implementation note:* placed in `view/agent/components/` (not `element/` as originally
     sketched) so the view→element dependency direction stays correct.

2. **Extract a terminal string from a result** — `terminalText(result): string | null`:
   prefer `stdout`(+`stderr`), else `output`, else `content`, when they are strings; return
   `null` for purely structured or empty results (so callers fall back to JSON).

3. **Wire it in (`CompactResult.tsx`):** the expanded branch renders
   `<TerminalOutput text={terminalText(result)}/>` when that's non-null, else the JSON `<pre>`.
   This single change fixes `Task`, `Agent`, and all `default` (unknown-tool, incl.
   `TaskOutput`) results at once.

4. **Optional polish (live + Bash parity):** `ChunkList` and `BashOutputViewer` currently render
   plain `<pre>`. They can adopt `AnsiLine` for color. **Caveat:** bashwrap currently
   **strips ANSI** before publishing chunks (`bash_wrap.rs` `strip_ansi`), so live chunks have no
   color to render today — enabling live color also requires bashwrap to *stop* stripping SGR
   (keep stripping cursor-movement/CR handling). Treat this as a follow-up, not part of the core
   fix.

### 4.3 Why not xterm.js in the feed
The term pane uses xterm.js, but it's a full interactive VT emulator (input, resize, WebGL,
scrollback buffers) — heavy and stateful for what the feed needs: static, colored, monospace
scrollback. `AnsiLine` already covers SGR coloring; a thin `TerminalOutput` wrapper is the
**efficient** path and keeps the feed DOM cheap and capped.

### 4.4 Tests — implemented
- `TerminalOutput`: multi-line split; ANSI line colorized; cap + hidden-marker beyond
  `MAX_TOOL_OUTPUT_LINES`.
- `terminalText`: picks `stdout`/`output`/`content`; returns `null` for `{a:1,b:2}` / empty.
- `CompactResult`: result with `{content:"line1\nline2"}` / `{stdout:…}` renders `TerminalOutput`
  (no JSON `<pre>`); a purely structured result still renders JSON.

---

## 5. Efficiency summary (the recommended minimal change set)

| # | Change | File(s) | Size |
|---|--------|---------|------|
| F1 | Trailing-redirect detection + `tee` rewrite at hook time | `agentmux-bashwrap/src/hook.rs` (+ tests) | ~1 scanner + grammar |
| F2a | `TerminalOutput` component (reuses `AnsiLine`, `capText`) | new `view/agent/components/TerminalOutput.tsx` | small |
| F2b | `terminalText()` helper + use in `CompactResult` expanded branch | new `terminal-text.ts` + `CompactResult.tsx` | a few lines |

F1 needs **no frontend work** (reuses the chunk pipeline). F2 needs **no backend work** (reuses
`AnsiLine`). Neither adds a dependency.

---

## 6. Risks / edge cases
- **F1 shell-parsing fragility** — mitigated by the conservative "bail to verbatim" stance; we
  only transform unambiguous trailing redirects and otherwise behave exactly as today.
- **F1 exit code** — must keep `set -o pipefail`; covered by a test.
- **F1 file bytes** — `tee` reproduces the redirect's bytes; the `2>&1` forms intentionally put
  both streams in the file (matching `> f 2>&1`). The no-`2>&1` form's file still gets stdout only.
- **F2 huge output** — `TerminalOutput` caps exactly like `CompactResult`/`BashOutputViewer`
  (`MAX_TOOL_OUTPUT_LINES`) so a large result can't bloat the conversation DOM.
- **F2 ANSI scope** — `AnsiLine` handles SGR only; cursor-movement sequences won't "move" but are
  harmless (rendered/dropped as text). Acceptable for scrollback.

## 7. Out of scope / follow-ups
- Live-chunk **color** (requires bashwrap to stop stripping SGR — §4.2.4).
- Tailing arbitrary redirect targets that F1 declines to rewrite (a file-watcher fallback) — only
  if real commands hit the bail path often.
- Streaming the persistent-shell-node (`ShellNode`) output through the same `TerminalOutput`.

## 8. Verification
- Build: `task build:backend` (+ `cargo test -p agentmux-bashwrap`), frontend `npm test`.
- Live: `task dev` → an agent runs `somecmd > out.log 2>&1` → confirm the feed streams the
  output live (F1) **and** `out.log` is written; run a `Task`/background tool → confirm its
  result renders as monospace terminal text, not a JSON blob (F2). Observe via `muxlog fe`.

## 9. Key file references
- `agentmux-bashwrap/src/hook.rs` — command rewrite (F1 insertion point + `tee_redirect_rewrite`)
- `agentmux-bashwrap/src/bash_wrap.rs` — exec/PTY capture, `strip_ansi`, model blob
- `frontend/app/view/agent/components/ToolOverlayLog.tsx:233-312` — `renderToolResultBody` switch
- `frontend/app/view/agent/components/CompactResult.tsx` — summary + terminal/JSON branch (F2)
- `frontend/app/view/agent/components/TerminalOutput.tsx` — terminal renderer (F2)
- `frontend/app/view/agent/components/terminal-text.ts` — result→terminal-string helper (F2)
- `frontend/app/element/ansiline.tsx` — `AnsiLine` (reused for terminal coloring)
- `frontend/app/view/agent/components/output-cap.ts` — `capText`, `MAX_TOOL_OUTPUT_LINES`
