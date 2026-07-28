# SPEC_TOOL_LOG_UNIVERSAL_ANIMATION_COLLAPSE_2026_07_27

## Problem

The tool-preview log (agent pane's expanded tool-call overlay, and the
persistent-shell-node panel) repeats lines during animated CLI output — most
visibly during package-manager installs, where every progress-bar/spinner
frame becomes its own permanent line instead of overwriting the previous one
in place.

This has already been partially fixed, twice, but both fixes are narrowly
scoped to **the animated character sitting alone at the start of its own
line**. Real-world spinners and progress bars almost never look like that —
they trail or lead static text on the same line
(`Installing dependencies... ⠋`, `Downloading pkg (45%)`, `[####    ] 42%`),
and that shape is exactly what both existing fixes miss. This spec
generalizes both layers so the fix covers the animated character (or
progress text) appearing **anywhere** in the line, not just isolated on its
own.

---

## Background / prior art

Two independent fixes already exist, at two different layers, each solving a
narrower sub-case:

### 1. Backend: `agentmux-bashwrap/src/bash_wrap.rs` (PR #1351)

- `collapse_cr` (lines 1415–1465) correctly collapses `\r`-driven overwrites
  — leading or mid-line — **within a single accumulated `pending` buffer**.
  This part is already general; the gap is what happens at the flush
  boundary around it.
- `pty_reader_loop` / `stream_reader`'s quiet-window flush (lines 1207–1247):
  on a quiet-window timeout, a `pending` buffer that starts with `\r` is
  stashed (`pending_cr_override`) so the next read can prepend and
  re-collapse it — but a buffer that does **not** start with `\r` is flushed
  immediately as a permanent `LineEvent`. The code's own comment says it
  plainly (lines 1230–1233):

  > "Non-`\r`-prefixed first frames (e.g. a tool that starts with `"frame1"`
  > then switches to `"\rframe2"` overwrites) are already flushed as regular
  > LineEvents by the time the `\r`-prefixed frames arrive; they cannot be
  > retroactively collapsed."

  This is the overwhelmingly common real pattern — print the static label
  once, then every subsequent update is `\r`-prefixed — and it's exactly the
  case that leaks a duplicate first line today.

- `pending_cr_line` (WPS publish path, lines 1467–1613) has the identical
  restriction: `line_str.strip_prefix('\r')` (line 1524) only triggers on a
  **leading** `\r`.

- `strip_ansi` (lines 1315–1389) explicitly documents the adjacent gap under
  "Things we DON'T handle yet (Phase γ territory)" (lines 1311–1313):
  **cursor positioning escapes within the same line** (`\x1b[<n>D`,
  `\x1b[<n>G`, `\x1b[2K`, `\x1b[<n>A`) are stripped as plain formatting, not
  recognized as overwrite signals equivalent to `\r`. Progress bars that use
  CSI repositioning instead of a literal `\r` byte (common in npm/yarn/cargo
  multi-line progress) are invisible to `collapse_cr` entirely.

### 2. Frontend: `frontend/app/view/agent/components/output-cap.ts` (lines 155–202)

`docs/specs/SPEC_TOOL_LOG_INPLACE_ANIMATION_2026_06_22.md` added
`collapseSpinnerChunks`, consumed by both `ToolOverlayLog.tsx:375`
(`ChunkList`, the tool-call preview) and `PersistentShellBlock.tsx:96` (the
persistent-shell-node panel) — this is shared logic across both surfaces.
That spec scoped itself explicitly to "tools that output frames as plain
`char\n` lines with no carriage return" (its own §Problem, lines 22–26) —
i.e., **the entire chunk, after trim, must equal exactly one glyph** from
`SPINNER_CHARS`:

```ts
if (SPINNER_CHARS.has(trimmed)) { ... }   // whole-chunk match only
```

Any frame where the spinner glyph trails or leads real text on the same line
never matches, and is pushed to `display` as a new permanent line — this is
the literal duplicated-line symptom in the tool preview. (Note: the original
spec's doc comment listed ASCII chars `- \ | /` in `SPINNER_CHARS`; the
current code excludes them, with an inline comment citing false-positive
risk. This spec keeps that exclusion — see Detection below.)

**Scope note:** the real xterm.js terminal pane (`termwrap.ts`) and the
dedicated install-progress modal (`AgentInstallModalPanel`) are unaffected —
both use a real VT100 emulator that already handles `\r`/CSI natively. This
bug is specific to the chunk-array preview renderer shared by
`ToolOverlayLog.tsx` and `PersistentShellBlock.tsx` via `output-cap.ts`.

---

## Research: how real tools solve this generally

Full survey in the task notification above; summary of the applicable
technique:

- **Full terminal emulators** (xterm.js, tmux, `pyte`/vt102-style headless
  emulators, VS Code's integrated terminal) maintain a screen buffer + cursor
  position and interpret the actual control stream — CR, LF, CUU
  (`\x1b[nA`), CHA (`\x1b[nG`), EL (`\x1b[K`/`\x1b[2K`), CUD/CUB/CUF. A row is
  only committed to scrollback when a linefeed actually scrolls it off the
  live viewport; nothing is appended on every write. This is the structurally
  correct approach and is what makes real terminals immune to this bug class.
- **CLI spinner/progress libraries** (`ora`/`cli-spinners`, `log-update`,
  `indicatif`, npm/yarn/cargo/docker's own progress UIs) universally use one
  of: bare `\r` + rewrite, `\r` + EL (`\x1b[2K`) + rewrite, or CHA
  (`\x1b[nG`) to reposition without erasing. Multi-progress-bar UIs (docker
  pull, yarn workspaces, cargo multi-crate) use CUU N + redraw N lines with
  EL on each — i.e. a **multi-line** in-place redraw, not just single-line.
- **Tools that skip real emulation and just append lines** (naive CI log
  viewers, chat/agent tools piping raw terminal bytes into a flat buffer)
  reproduce this exact bug; the documented fix pattern (e.g. Roo-Code
  #2561) is "only show the content after the last `\r` on each accumulated
  line" — i.e., don't treat `\r` as a line delimiter, treat it as an
  in-place-edit marker.
- **Fallback heuristic** when literal escape-sequence parsing is
  incomplete or unavailable: normalized similarity between consecutive
  lines (strip digits/spinner glyphs/whitespace, compare) — used by
  near-duplicate log collapsers (e.g. `stutterlog`, Damerau-Levenshtein
  based) as a second line of defense for content the structural parser
  doesn't recognize as an overwrite.

Sources: xterm.js scrollback/ED issues, `pyte` docs, Roo-Code #2561,
`sindresorhus/log-update` + `ansi-escapes`, `console-rs/indicatif`,
`cli-spinners` frame data, `stutterlog`.

**Takeaway for this codebase:** we are not building a full terminal emulator
(that's what `termwrap.ts` is for elsewhere in this app) — the tool-preview
chunk renderer is intentionally a lighter-weight, log-style view. So the
right generalization here is a **hybrid**: extend the backend's already-real
`\r`/CSI parsing to not lose the first frame at flush boundaries and to
recognize CSI overwrite sequences (structural fix, high confidence), plus
extend the frontend's chunk-collapsing to recognize "this chunk is a
redraw of the previous one" by content similarity, not just exact
whole-line spinner-glyph match (heuristic fallback, catches whatever the
backend doesn't normalize away).

---

## Approach

### A. Backend — `agentmux-bashwrap/src/bash_wrap.rs`

**A1. Stop permanently flushing the first frame of an about-to-be-overwritten sequence.**

At the quiet-window flush point (`pty_reader_loop` / `stream_reader`,
~lines 1207–1247), a `pending` buffer that does not start *or* end with `\r`
is currently flushed immediately. Instead, hold it as a **speculative
pending line** for one additional quiet-window tick (same debounce interval
already used elsewhere in this file) rather than flushing outright:

- If the *next* read begins with `\r`, treat it as an overwrite of the held
  speculative line (same collapse path `collapse_cr` already implements for
  the leading-`\r`-pending case) instead of appending a new line.
- If the next read does *not* begin with `\r`, or the quiet window elapses
  again with nothing new, flush the held line normally — this preserves
  today's behavior for genuinely static output (no added latency beyond one
  extra quiet-window tick, which only fires on already-idle streams).

This directly closes the gap called out in the existing code comment
(lines 1230–1233) and requires no new data structures — it's a small
extension of the existing `pending_cr_override` stash to also apply when the
**previous** flush (not just the current pending buffer) was a candidate.

**A2. Recognize CSI overwrite sequences as equivalent to `\r`.**

Extend `collapse_cr` (or add a sibling pass ahead of it) to treat these as
line-overwrite markers, matching the "Phase γ" gap `strip_ansi` already
documents (lines 1311–1313):

| Sequence | Meaning | Treat as |
|---|---|---|
| `\x1b[<n>D` (CUB) | cursor back n cols | overwrite from column `cur - n` |
| `\x1b[<n>G` (CHA) | cursor to absolute column n | overwrite from column n |
| `\x1b[2K` / `\x1b[K` (EL) | erase line (whole/to-end) | clears pending buffer content from cursor |
| `\x1b[<n>A` (CUU) + redraw | cursor up n lines | overwrite of the last n *already-flushed* preview lines, not just `pending` — see A3 |

A single-line CSI overwrite (CUB/CHA/EL without CUU) can be handled entirely
within the existing `pending`-buffer model: apply the cursor move against
the buffer's current column position, then continue accumulating bytes as
overwrites at that position, same as `collapse_cr` already does for `\r`.

**A3. Multi-line CUU-based redraw is out of scope for this pass.**

CUU N (cursor up N lines) implies overwriting N *already-emitted* lines —
that requires the backend to hold a small rolling window of recent
`LineEvent`s it can retract/replace, which is a materially bigger change
(the wire protocol currently only supports appending `LineEvent`s, not
retracting them). Flag this as a documented follow-up (multi-progress-bar
tools like docker pull / yarn workspaces) rather than attempting it here;
A1+A2 already cover the dominant single-line case this spec was scoped to
generalize (spinner/progress trailing or leading static text on one line).

### B. Frontend — `output-cap.ts`

Generalize `collapseSpinnerChunks` from "whole chunk is exactly one spinner
glyph" to "this chunk is a redraw of the previous displayed line," using a
similarity check as the fallback the backend's structural fix won't fully
replace (agents run tools whose output arrives however the shell/PTY layer
chunked it; some near-duplicate frames will still slip through as separate
chunks even after A1/A2).

```ts
// Existing: exact single-glyph match.
const SPINNER_CHARS = new Set([...]);  // unchanged

// New: does `next` look like an in-place redraw of `prev`?
function looksLikeRedraw(prev: string, next: string): boolean {
    if (prev === next) return false; // identical repeats aren't animation, handled elsewhere
    const stripPrev = normalizeForCompare(prev);
    const stripNext = normalizeForCompare(next);
    if (stripPrev === stripNext) return true; // differs only in glyph/%/count
    return levenshteinRatio(stripPrev, stripNext) >= REDRAW_SIMILARITY_THRESHOLD;
}

// Strip spinner glyphs, digits, and percentage/progress-bar filler so
// "Installing... ⠋" / "Installing... ⠙" / "Downloading (45%)" / "(46%)"
// normalize to the same or a near-identical string.
function normalizeForCompare(s: string): string {
    return s
        .replace(SPINNER_CHAR_RE, "")
        .replace(/\d+%?/g, "#")
        .replace(/[#=\-\s]{3,}/g, "#") // progress-bar fill runs
        .trim();
}
```

Extend the collapse loop so a run is a **redraw run** if each chunk either
whole-glyph-matches `SPINNER_CHARS` (existing case, kept as a fast path) *or*
`looksLikeRedraw(lastFrame, trimmed)` is true against the previous frame in
the run. Same freeze/live-slot semantics as today: a trailing run stays a
live-updating slot while streaming, a completed run freezes on its last
frame. `REDRAW_SIMILARITY_THRESHOLD` starts at `0.82` (empirically: a
spinner-glyph-only diff or a 2–3 digit percentage change scores well above
this; unrelated consecutive lines score well below it) — tune against real
captured install logs during implementation rather than treating this as
final.

Apply identically in `PersistentShellBlock.tsx` (same shared-logic point
noted in Background).

---

## Detection summary

| Case | Layer | Mechanism |
|---|---|---|
| Bare `\r` overwrite, leading or mid-buffer, within one flush | Backend | `collapse_cr` (already works) |
| Bare `\r` overwrite whose *first* frame has no leading `\r` | Backend | **A1** — speculative hold-and-merge at flush boundary |
| CSI CUB/CHA/EL single-line overwrite | Backend | **A2** — treat as `\r`-equivalent in `collapse_cr` |
| CSI CUU multi-line redraw (docker/yarn-style) | — | Out of scope, documented follow-up |
| Whole chunk is exactly one spinner glyph | Frontend | `SPINNER_CHARS` exact match (existing, kept) |
| Spinner glyph trailing/leading static text on the same line | Frontend | **B** — `looksLikeRedraw` similarity fallback |
| Percentage/progress-bar text changing per frame | Frontend | **B** — `normalizeForCompare` digit/fill stripping |
| Genuinely different consecutive lines | Frontend | Similarity below threshold → not collapsed (unchanged) |

---

## Edge cases / risks

- **False-positive collapse of legitimately similar but distinct lines**
  (e.g. two consecutive `npm WARN` lines about different packages that
  happen to share most characters). Mitigated by requiring the
  normalized-diff to specifically be digits/percent/fill-runs/spinner
  glyphs — not an unbounded fuzzy match — plus the empirically-tuned
  threshold. This mirrors why ASCII spinner chars (`- \ | /`) are already
  excluded from `SPINNER_CHARS` today (false-positive risk on legitimate
  single-char lines like a lone `-` bullet).
- **A1's extra quiet-window tick** adds latency only to the already-idle
  tail of a stream (nothing to do with steady-state throughput), and only
  when a line hasn't been confirmed static yet.
- **A3 (multi-line CUU) deliberately deferred** — flag as follow-up so it
  isn't silently forgotten; docker pull / yarn workspace installs will still
  show some duplication after this pass, just less than today (their
  per-line spinner/percentage churn is still caught by A1/A2/B; only the
  N-lines-at-once redraw shape is unhandled).

---

## Files to change

- `agentmux-bashwrap/src/bash_wrap.rs` — A1 (flush-boundary hold-and-merge), A2 (CSI-as-overwrite in `collapse_cr`)
- `frontend/app/view/agent/components/output-cap.ts` — B (`looksLikeRedraw` / `normalizeForCompare`, generalize `collapseSpinnerChunks`)
- `frontend/app/view/agent/components/ToolOverlayLog.tsx` — consumes the generalized collapse (no logic change expected, same call site)
- `frontend/app/view/agent/components/PersistentShellBlock.tsx` — same collapse, same call site
- `docs/specs/SPEC_TOOL_LOG_INPLACE_ANIMATION_2026_06_22.md` — cross-reference this spec as the generalization; leave as historical record of the initial narrower fix rather than rewriting it

---

## Test plan

- Backend unit tests in `bash_wrap.rs` (alongside existing
  `collapse_cr_leading_spinner_frames_collapse` /
  `collapse_cr_trailing_spinner_frames_collapse`):
  - First frame has no leading `\r`, subsequent frames do → single collapsed
    `LineEvent`, not two.
  - CSI CHA/CUB/EL sequence mid-line → treated as overwrite, not stripped
    into noise.
  - Genuinely static output (no `\r`/CSI at all) → unchanged flush timing.
- Frontend unit tests for `output-cap.ts`:
  - `"Installing deps... ⠋"` → `"Installing deps... ⠙"` → collapses to one
    live slot.
  - `"Downloading (12%)"` → `"Downloading (45%)"` → `"Downloading (100%)"` →
    collapses, freezes on final frame.
  - Two unrelated consecutive lines with coincidental partial overlap → NOT
    collapsed.
  - Existing whole-glyph spinner case → unchanged (regression guard).
- Manual: run an actual `npm install` / `cargo build` as an agent Bash tool
  call, inspect the expanded tool preview and the persistent-shell panel —
  confirm a single settling line/slot instead of a repeated stack, for both
  a real spinner-with-prefix-text tool and a percentage-progress tool.
