# SPEC: Robust highlighting for Bash tool panels in the agent pane

**Status:** active. Steps 1-3 of §5 (POSIX tokenizer, sync render and theme, streaming header) shipped in #4314. Step 5's heredoc bodies are built (§3.2); its inline scripts, and steps 4, 6 and 7 (PowerShell and cmd tokenizers, output rendering, row highlighting and danger markers), remain.
**Date:** 2026-10-04
**Author:** AgentX
**Requested by:** the repo owner ("when hovering over agent pane tools, we get
floating panels over the preview. A lot of them are Bash commands. Can we get
robust highlighting for them?")
**Scope:** `frontend/app/view/agent/components/` (the Bash result viewer, the
streaming chunk list, the tool row header) plus a new pure-TypeScript
highlighting module. No backend change.
**Builds on:** `SPEC_TOOL_OVERLAY_CODE_HIGHLIGHTING_2026_04_14.md` (Shiki for
Read/Edit/Bash command), `SPEC_UNIFIED_TOOL_HOVER_OVERLAY_2026_05_13.md`,
`SPEC_TOOL_HOVER_CONSOLIDATION_2026_05_28.md`,
`SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md`.

---

## 1. Problem

Hovering a tool row opens the tool panel over the transcript. Most of those
panels are Bash. What a Bash panel shows today:

| Part | Finished call | Streaming call |
|---|---|---|
| Command | Shiki `bash`, async (plain text first, colours swap in later) | **not shown** (the chunk list has output only) |
| Output | plain `<pre>`, one colour; stderr is all red | plain `<pre>` per chunk |
| Header row (collapsed) | plain text, ellipsis-truncated | same |
| Exit status | pill + "Exit code: N" line | none until done |

Specific gaps, each seen in real agent sessions:

1. **Shell commands are more than `bash`.** Agents on Windows run PowerShell
   and cmd through the same tool, and run `python - <<'EOF'`, `node -e '...'`,
   `jq '...'`, `git commit -m "$(cat <<'EOF' ... EOF)"`, `cat > f.ts <<EOF`.
   Shiki's `bash` grammar colours the heredoc body as one string, so the
   Python or TypeScript inside is a wall of one colour. PowerShell given to
   the `bash` grammar comes out wrong (wrong quoting, `$_`, `-Property`).
2. **Long chains are unreadable.** `cd x && cargo test -p a 2>&1 | tail -40
   && git status` wraps as one block. Nothing marks where one command ends and
   the next begins, or which word is the program and which are flags.
3. **Output carries meaning that is thrown away.** ANSI colour from cargo,
   pytest, git, eslint and ripgrep is shown as raw `ESC[31m` noise or plain
   text. Unified diffs, JSON, stack traces, `path:line:col` locations and
   test-result lines (`FAILED`, `error[E0425]`, `ok`) all render grey.
   `parseAnsi` (`element/install/ansi.ts`) and `ansiline.tsx` already exist
   but the tool panel uses neither.
4. **Flash on every open.** `HighlightedCode` renders plain text, then swaps
   in Shiki HTML after an async import. Hover is a transient surface, so users
   see the flash again each time a command is not yet in the cache.
5. **Dark theme only.** `HighlightedCode` and `DiffViewer` hard-code
   `github-dark-high-contrast`. On a light theme the command is near-invisible
   white-on-light (the surrounding `.agent-bash-cmd-code` also forces
   `color: var(--accent-color)` over Shiki's inline colours).
6. **Streaming panels lose the command.** The first thing a user wants while
   a call runs is "what is this running", and the panel shows only output
   (or a spinner) for exactly that period.
7. **Dangerous commands look like every other command.** `rm -rf`, `git push
   --force`, `git reset --hard`, `curl ... | sh`, `DROP TABLE` read the same as
   `ls`. The agent pane already treats these as red flags elsewhere (permission
   prompts); the panel should not hide them.

## 2. Goals and non-goals

**Goals**

- G1. Commands are tokenised and coloured correctly for bash/sh/zsh,
  PowerShell and cmd, including heredoc bodies in their real language.
- G2. A compound command is visually segmented: program names, flags,
  arguments, operators, redirects and substitutions are distinguishable.
- G3. Output is rendered with ANSI colours, and recognised formats (diff,
  JSON, locations, test/compiler diagnostics) get light structure.
- G4. No flash: first paint is already highlighted for anything the panel can
  tokenise synchronously; Shiki only upgrades it.
- G5. Theme-aware (light and dark), using the pane's existing tokens.
- G6. Works while streaming, with bounded cost per chunk.
- G7. Dangerous-command markers, derived from tokens, not string search.
- G8. Never wrong in a harmful way: highlighting must not hide, reorder or
  alter text. Copy gives the exact original bytes.

**Non-goals**

- Executing, parsing for policy decisions, or sandboxing commands. This is
  presentation only; permission logic keeps its own parser.
- Changing the panel's size, hover timing, or scroll-follow behaviour (see the
  hover and scroll specs above).
- Highlighting PtyShell/terminal panes (xterm renders its own colours).
- A full shell parser. A tolerant tokenizer is enough; it must never throw.

## 3. Design

A new module `frontend/app/view/agent/components/shell-highlight/` holds pure
functions (no DOM, no Solid). Components consume its output.

### 3.1 Command tokenizer

`tokenizeShell(command, flavor): Token[]` where `Token = { start, end, kind }`
(offsets into the original string, so text is never copied or altered).

- **Flavor detection** `detectShellFlavor(command, hints)`: PowerShell when
  the command starts with `powershell`/`pwsh`, contains `Get-`/`Set-`/`$_`/
  `| Where-Object`, or uses `-Name value` cmdlet params; cmd for `dir`,
  `%VAR%`, `if exist`, `^` continuations; otherwise POSIX. `hints` carries the
  host OS and connection (a WSL or SSH pane is POSIX). Ambiguity falls back to
  POSIX.
- **Kinds:** `program`, `subcommand` (second word of `git`, `cargo`, `npm`,
  `docker`, `gh`, `kubectl`, ... from a small table), `flag`, `flag-value`,
  `string`, `variable`, `substitution`, `operator` (`| && || ; &`), `redirect`
  (`> >> 2>&1 <`), `path`, `url`, `number`, `comment`, `heredoc-marker`,
  `heredoc-body`, `env-assign` (`FOO=bar cmd`), `punct`, `plain`.
- **Segments:** a pass over operator tokens yields `Segment[]` (one per
  simple command) so the view can give each its own program accent and, when
  the command is multi-line or has more than three segments, put a faint
  separator at operators.
- **Tolerance:** unterminated quotes, unmatched `$(`, binary junk and
  100 KB commands must return tokens covering the input without throwing.
  Anything the tokenizer is unsure about is `plain`. Property tests: for any
  input, the concatenation of token ranges equals the input.

### 3.2 Embedded languages

`embeddedBodies(tokens, command)` returns `{ start, end, lang }` for:

- heredoc bodies (`shell-highlight/embedded.ts`), language from, in order:
  1. the program that reads it (`python`, `node`, `deno`, `bash`, `sh`,
     `psql`, `sqlite3`, `jq`, `kubectl`, `ruby`, `perl`, `pwsh`, ...);
  2. the file it is written to, through `detectLanguage` (`cat > a.ts <<EOF`,
     `cat >> f <<EOF`, `tee [-a] f <<EOF`; tee's operand wins over a
     `>/dev/null` after it);
  3. commit messages and PR/issue bodies as `markdown`: the program is `git`,
     `gh`, `gh-agent` or `glab`, directly (`--body-file - <<EOF`) or around a
     `cat` (`git commit -m "$(cat <<'EOF' ... EOF)"`);
  4. the delimiter's name (`<<'PY'`, `<<JSON`, `<<SQL`);
  5. what the heredoc is piped into (`cat <<EOF | kubectl apply -f -`);
  6. a shebang on the body's first line;
- inline scripts (not built yet): `python -c '...'`, `node -e "..."`,
  `sh -c '...'`, `pwsh -Command "..."`, `jq '<filter>'`, `awk '<prog>'`,
  `sed 's/..'`, and `git commit -m "..."` as `markdown`.

A survey of 5,790 heredocs in agent transcripts on one host (2026-10-05):
48% are read by a known program, 13.5% are written to a file with a known
extension, about 35% are commit messages or PR bodies, 0.1% are told only by
the delimiter, and about 3% stay unknown. 99% have a quoted delimiter.

Shell bodies are rendered with the sync tokenizer, so they are coloured on
first paint. Bodies in other languages are highlighted by Shiki and spliced
into the token stream. Unknown language leaves the body as `heredoc-body`,
in the plain text colour (it used to be string green, which turned a whole
pasted file green). Nesting is one level deep (a heredoc inside `$(...)`
inside `-m` is the common case and is handled; deeper nesting stays flat).

### 3.3 Rendering without a flash

Two layers:

1. **Sync layer (always available, no network, no WASM).** The tokenizer's
   kinds map to CSS classes (`.sh-program`, `.sh-flag`, ...) coloured from the
   theme tokens in 3.5. This renders on first paint, so there is never an
   uncoloured frame.
2. **Shiki upgrade (optional).** Only for embedded bodies in other languages
   (3.2), where a real grammar matters. It swaps a body's `<span>` run in
   place, same text, same layout box, so nothing shifts. If Shiki is slow,
   fails, or the body is over the existing size cap, the sync colours stay.

The command itself no longer goes through whole-command `codeToHtml(bash)`;
the Shiki `bash` grammar is retired from this panel. `HighlightedCode`
keeps serving Read/Write/Edit unchanged.

A module-level cache (key: flavor + command) keeps token arrays, so re-hover
is free. Caps: tokenize at most 20 000 chars (rest renders `plain`), highlight
embedded bodies at most 200 KB (the existing `CAP_BYTES`).

### 3.4 Output

`renderOutput(text, kind)` returns a line model `Line = Span[]` computed once
per chunk (not per render), in this order:

1. **ANSI SGR** -> spans (bold, dim, italic, underline, 16/256/truecolor fg and
   bg, reverse). Extend `parseAnsi` (currently bold + 16 fg only) rather than
   adding a second parser; keep it in `element/install/ansi.ts`'s successor
   under `element/ansi/` and make the install log use the same code. Other
   escapes (cursor movement, OSC titles) are dropped. Carriage-return
   progress lines collapse to their last state (reuse `createSpinnerCollapser`).
2. **Format sniffing**, on complete output only (finished call), by first
   non-empty lines: unified diff (`diff --git`, `@@`), JSON (`{`/`[` that
   parses), `git status`/`git log --oneline`. Streaming output gets only the
   per-line rules below. Sniffing never rewrites text; JSON is coloured as
   written, not re-indented.
3. **Per-line rules**, applied only to spans that ANSI did not already colour:
   - diff: `+`/`-` line tint, `@@` header, `diff --git` file header;
   - locations: `path:line[:col]` and `path(line,col)` become a `location`
     span; click opens it in an editor pane at that line (the editor pane
     already accepts line targets from the Read tool's file-range code);
     only paths that exist relative to the call's cwd are linked, checked
     lazily on hover of the span, never up front;
   - diagnostics: lines starting `error`/`warning`/`note`/`FAILED`/`panicked`/
     `Traceback`/`npm ERR!` get a left-margin marker colour (not whole-line
     recolour, so ANSI the tool already emitted still wins);
   - URLs: existing `LinkifiedText` behaviour is kept.
4. **stderr** keeps its own accent, but as a left rule plus label, not
   all-red text, so ANSI colours inside stderr stay readable. Exit status
   styling is unchanged.

All of this is one pass over capped text (`MAX_TOOL_OUTPUT_LINES`,
`capChars`), done in a `createMemo` per chunk list; chunk identity tracking
stays the same as today so streaming does not re-render old lines.

### 3.5 Theme and CSS

Sync-layer classes take their colours from each theme's terminal palette
(`--term-green`, `--term-blue`, ...), plus `--accent-color` for programs and
`--link-color` for URLs. Every theme already defines the palette against its
own background, so light and dark themes stay legible without a second set of
tokens. The forced `color: var(--accent-color)` on `.agent-bash-cmd-code` is
removed. Embedded-body Shiki output (step 5) is highlighted with
`github-dark-high-contrast` and `github-light-high-contrast` at once
(`codeToTokens` with `themes` and `defaultColor: false`): each run carries
`--shiki-dark` and `--shiki-light`, and the stylesheet picks one by
`[data-theme-polarity="light"]`, so a theme flip needs no re-highlight. Runs
are rendered as spans with text children and only `--shiki-*` style
variables, never through `innerHTML`. The ANSI
palette comes from the existing `text-ansi-*` classes.

### 3.6 Streaming panels show the command

The streaming `ChunkList` branch gains the same `$ command` header as the
finished viewer (a shared `BashCommandView`), so the command is visible from
the first frame. This changes only what is rendered above the chunks, not
the scroll-follow or FLIP logic in `ToolOverlayLog.tsx`.

### 3.7 Collapsed row

The one-line header's detail text uses the sync layer too (program in accent,
flags dimmed, the rest default), single-line, ellipsis as today. It must not
re-measure on hover; classes only, no layout-affecting differences (no bold
shifting widths).

### 3.8 Danger markers

`classifyDanger(segments)` marks a segment when its tokens match a small,
explicit table: `rm` with `-r`/`-f` (any flag order or long form),
`git push --force|-f`, `git reset --hard`, `git clean -f`, `git checkout .`,
`chmod -R`, `chown -R`, `dd of=`, `mkfs`, `> /dev/sd*`, `curl|wget ... | sh|bash`,
`Remove-Item -Recurse -Force`, `format`, `DROP|TRUNCATE` in an embedded SQL
body, and any segment run through `sudo`. The marker is the `--sh-danger`
colour on the program and the offending flag, plus a `title` naming the reason.
It is advisory: no blocking, no new prompts. The table lives in one file with
a test per row, and a false positive is a visual annoyance, never a behaviour
change.

## 4. Safety and correctness constraints

- Text is never altered. Every span is a range into the original string;
  tests assert `spans.map(text).join("") === input` for the command and for
  each output line (post ANSI-stripping).
- No `innerHTML` from untrusted text. The sync layer builds elements with
  `textContent`. Shiki's HTML (already trusted-shape: `<span>` with
  `class`/`style`) stays the only `innerHTML` path, as in `HighlightedCode`.
- Secrets: highlighting does not add any new place the command text is
  stored, logged or sent. Caches are in-memory and bounded (LRU, 200 entries).
- Performance: tokenizing is linear and allocation-light; budget under 2 ms
  for a typical 200-char command and under 20 ms for the 20 000-char cap.
  Output work is per new chunk only.

## 5. Implementation plan (separate PRs)

1. **Tokenizer + tests.** `shell-highlight/` with POSIX tokenizer, segments,
   round-trip property tests, a fixture corpus of 100 real commands (taken
   from transcripts, scrubbed of anything private).
2. **Sync render + theme tokens.** `BashCommandView` using the sync layer for
   the finished viewer; remove the accent override; light-theme check.
   Screenshots in the PR for dark and light.
3. **Streaming header.** Show the command in the streaming branch.
4. **PowerShell and cmd flavors.**
5. **Embedded bodies** via Shiki, heredoc/inline-script detection.
6. **Output: ANSI extension, format sniffing, locations, diagnostics.** Move
   `parseAnsi`; install log keeps passing its tests.
7. **Collapsed-row highlighting and danger markers.**

Each PR is independently shippable; 1-3 give the biggest visible gain.

## 6. Tests

- Tokenizer: round-trip over the fixture corpus and fuzzed input (random
  bytes, unterminated quotes, deep nesting); golden token kinds for ~40
  representative commands (chains, pipes, redirects, env-assign, `$(...)`,
  heredocs with each embedded language, PowerShell pipelines, cmd).
- Flavor detection table tests, including the ambiguous cases falling back
  to POSIX.
- Embedded bodies: language selection order; unknown language stays flat.
- Danger table: one positive and one near-miss negative per row (`rm -r dir`
  vs `rm file`; `git push` vs `git push --force-with-lease`, which is
  deliberately not flagged).
- Output: ANSI SGR matrix incl. 256/truecolor, `\r` collapse, dropped
  non-colour escapes; diff/JSON sniffing; location parsing for Windows
  (`C:\a\b.rs:12:3`) and POSIX paths, and `file(12,3)`.
- Component tests (vitest + jsdom): first render is already coloured with no
  Shiki loaded; Shiki upgrade does not change `textContent`; theme flip
  re-highlights; streaming header present; stderr label.
- Layout-read gate: no `getBoundingClientRect`/`scrollHeight` reads added
  (CI gate); no new spawns.

## 7. Open questions

1. Should the danger markers also appear on the collapsed row (3.7), or only
   inside the panel? Proposal: both, since the row is what is on screen when
   the call is auto-approved.
2. Location links: open in an editor pane (proposed), or copy the path? The
   Read tool already opens editor panes, so the first needs no new surface.
3. Is a per-user "plain output" setting wanted for people who find colour
   noisy? Proposal: no setting in the first version; revisit on feedback.
