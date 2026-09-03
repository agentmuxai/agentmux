# Analysis: tool-preview code still reads over-indented, and some lines wrap

**Date:** 2026-09-02
**Author:** Agent4
**Repo state:** `agentmuxai/agentmux` main @ `01cd708e`
**Method:** static read of the render path + CSS cascade, arithmetic on the
tab-stop behaviour, and **measurement against real stored transcripts**
(`~/.agentmux/shared/agents/transcripts/filestore.db`, opened read-only) to
confirm the actual `Read` result format and the actual indentation mix. Not
verified by eye in a live pane — see §7.

**Prior work being followed up:** PR #2785 / commit `aa1d0a14`,
*"fix(agent-pane): render tool-preview tabs at 2 spaces instead of the inherited
4"* — a one-line CSS change adding `tab-size: 2` to `.agent-tool-panel`
(`frontend/app/view/agent/styles/_document-nodes.scss:393`), overriding the
app-wide `tab-size: 4` in `frontend/app/reset.scss:26`. Documented as a
follow-up inside `docs/specs/SPEC_TOOL_PREVIEW_DEDENT_2026_08_08.md`.

---

## 0. Summary

The report is "a lot of indentation still, but some lines are word-wrapped."
Both halves are real, and they are **two different problems on two different
sets of surfaces**:

1. **The indentation.** `tab-size: 2` can only ever affect *tab*-indented
   content, and no CSS property changes how wide a space renders. Measured on a
   real Read of a repo source file: **0 of 127 indented lines used tabs** — so
   the 08-24 fix was inert on that preview, and on essentially every preview of
   this codebase. Separately, `dedent.ts` is a **no-op for the two most common
   preview shapes** (a whole-file Write, and a Read that starts at the top of a
   file — 58 of that same Read's 200 lines sit at column 0, which forces the
   common prefix to empty), and is **not applied at all** to Bash output,
   Grep/Glob results, or streaming chunks. Net effect on that sample: the
   deepest lines render at **column 24** and nothing in the current pipeline
   reduces that.

2. **The wrapping.** Code previews (Read / Write / Edit) use `white-space: pre`
   and never wrap. Bash output, Grep/Glob results, and expanded JSON use
   `pre-wrap` **plus `overflow-wrap: anywhere`** and always wrap, breaking
   mid-identifier. Both live in the same `.agent-tool-panel`, so which behaviour
   you get depends on which tool produced the block. The surfaces that wrap are
   *the same surfaces that get no dedent* — so those blocks show full original
   indentation **and** wrap, which is the worst combination and almost certainly
   what prompted this report.

There is also a **regression introduced by the 08-24 fix itself** (§2): it made
the Read line-number gutter ragged at the line-9→10 boundary, which it was not
before.

---

## 1. Why `tab-size: 2` didn't move the needle

`tab-size` sets the rendered width of a **U+0009 TAB character**. It has no
effect on U+0020 SPACE.

I pulled a real `Read` of a Rust source file out of the transcript store
(`agentmux-srv/src/server/identity_handlers.rs`, `limit: 200`) and counted how
its 200 result lines are actually indented:

```
lines with TAB indent:   0
lines with SPACE indent: 127
leading-whitespace-width histogram (columns):
  0 → 58 lines   4 → 58   8 → 6   12 → 8   16 → 32   20 → 23
```

**Zero tab-indented lines.** For this preview — and for every `.ts`/`.tsx`/
`.scss`/`.rs` file in this repo, plus most Python, Go and JSON — `tab-size` has
no effect whatsoever, at any value.

Meanwhile the deepest lines carry **20 columns** of leading space, and the Read
gutter adds 4 more (§2), so the deepest lines start at **column 24**. In a
narrow agent pane that is most of the usable width, spent on indentation that
the preview can't even show the enclosing scopes for. That is the reported
symptom, measured.

So the 08-24 fix is correct for what it claims (tab-indented previews no longer
read wider than space-indented siblings) and simply does not intersect the
general "too much indentation" complaint. Nothing was wrong with the change; the
scope was narrower than the symptom.

**Implication:** if the goal is "previews should not waste half their width on
indentation", it cannot be solved in CSS. It has to be solved in the text that
gets handed to the renderer.

---

## 2. Regression: the 08-24 fix made the Read gutter ragged

Read content lines are `<N>\t<code>` — the line number, a **literal tab**, then
the source line. Confirmed verbatim from a `tool_result` frame in the transcript
store, **with no left-padding on the number**:

```
1\t// Copyright 2026, AgentMux Corp.
2\t// SPDX-License-Identifier: Apache-2.0
3\t
...
9\t//!   * `auth.poll`            — `PollProviderAuth`
10\t//!   * `auth.submitcallback`  — `SubmitAuthCallback`
```

Note `9\t` then `10\t` — the number is left-aligned, so the character count
before the tab **grows with the line number**. (Had the CLI right-aligned to a
fixed width, `cat -n` style, this whole finding would not exist.)

`stripCommonIndentNumbered` deliberately preserves that prefix verbatim
(`frontend/app/view/agent/components/dedent.ts:134-149`) and dedents only the
code portion.

That gutter tab is subject to the same `tab-size`. Because a tab advances to the
*next tab stop*, the column the code starts at depends on how many digits the
line number has:

| line number | digits | code starts at, `tab-size: 4` | code starts at, `tab-size: 2` |
|---|---|---|---|
| 1 – 9 | 1 | col 4 | **col 2** |
| 10 – 99 | 2 | col 4 | col 4 |
| 100 – 999 | 3 | col 4 | col 4 |
| 1000 – 9999 | 4 | col 8 | col 6 |

Scanning every line number 1–19999, the code's start column shifts at:

- `tab-size: 4` → **only at line 1000**
- `tab-size: 2` → **at line 10, and again at line 1000**

So every Read preview that spans the 9→10 boundary — i.e. essentially every read
of a file head, and every short file — now has a **2-column step in its left
edge partway down the block**. The transcript sample above is exactly such a
case: its lines 1–9 render two columns left of its lines 10+. Before #2785 that
block was flush. This is a new raggedness the change introduced.

`tab-size: 4` is strictly better *for the gutter*; `tab-size: 2` is better *for
relative indentation in a tab-indented file*. The two goals are in direct
conflict as long as the gutter is rendered as a tab inside the same `<pre>`.
§6.1 resolves the conflict by removing the gutter from the highlighted text
entirely, which makes the `tab-size` value stop mattering for the gutter at all.

---

## 3. Where dedent does and doesn't run

`dedent.ts` strips the **literal longest common leading-whitespace prefix**
across the displayed lines. Three cases where that is correctly a no-op, and
which together cover most previews:

| Case | Why dedent does nothing | Where |
|---|---|---|
| **Write of a whole file** | Whole files start at column 0, so the common prefix is `""`. Spec §2 acknowledges this ("Dedent is a natural no-op"). | `ToolOverlayLog.tsx:529` |
| **Read starting at the top of a file** | Same reason — any column-0 line in the visible range forces the common prefix to `""`. Only *mid-file* reads (`offset`) benefit. **Measured on the real Read in §1: 58 of its 200 lines sit at column 0, so the common prefix is `""` and dedent strips nothing at all** — while 23 lines sit at column 20. | `ToolOverlayLog.tsx:471` |
| **Mixed tabs and spaces at the same depth** | Deliberate, spec §3.3: the comparison is a literal string prefix, so `"\tfoo"` and `"    bar"` share nothing and dedent declines rather than guessing a tab width. | `dedent.ts:64-81` |

And three surfaces where it is **not wired up at all**, by explicit scope
decision in spec §2's table:

- **Bash** — "command output has no file-indentation semantic".
- **Grep / Glob** — "each match line is independent, so 'common' indent across
  unrelated lines is semantically weak." Listed as a candidate follow-up;
  never done.
- **Streaming chunks (`ChunkList`)** — dedent needs the full text to compute a
  prefix, so it can't run mid-stream.

Grep is the notable one: its result lines *are* code, they carry their full
original file indentation, and they are the case a user is most likely to be
looking at when they say "a lot of indentation".

---

## 4. Wrapping is inconsistent across surfaces inside one panel

Every one of these renders inside the same `.agent-tool-panel`:

| Surface | Class | `white-space` | Wraps? | Dedented? |
|---|---|---|---|---|
| Read / Write code | `.agent-highlighted-code` | `pre` (`_document-nodes.scss:666`) | **no** — h-scroll | yes |
| Edit diff (Shiki) | `.agent-diff-highlighted-body` — a `<pre>`, UA default | `pre` | **no** | yes (shared prefix) |
| Streaming chunks | `.agent-tool-log-line` | `pre` (`_tool-overlay-portal.scss:62`) | **no** | no |
| Bash command row | `.agent-bash-cmd-code` | `pre-wrap` + `word-break: break-all` (`:805`, `:809`) | **yes, mid-token** | n/a |
| Bash output, and any result with a string body | `.agent-terminal-output > div` | `pre-wrap` + `overflow-wrap: anywhere` (`:1772-1773`) | **yes** | no |
| Grep/Glob & other structured results, expanded | `.agent-tool-compact-json` | `pre-wrap` + `overflow-wrap: anywhere` (`:1444-1445`) | **yes** | no |
| Agent / Task / Workflow result text | `.agent-tool-agent-result` | `pre-wrap` (`:463`) | **yes** | no |

Three consequences:

**4.1 — The wrapping surfaces are exactly the non-dedented ones.** Bash output,
Grep results, and compact JSON show full original indentation *and* wrap. When a
line wraps, its continuation restarts at column 0, destroying the visual
indentation that the rest of the block still carries. That reads as "lots of
indentation, and ragged wrapping" — the reported symptom, precisely.

**4.2 — `overflow-wrap: anywhere` / `word-break: break-all` break mid-identifier.**
On `.agent-bash-cmd-code` this is deliberate and commented ("Force breaks inside
long tokens (long paths, base64, etc.)"). On `.agent-terminal-output` and
`.agent-tool-compact-json` it means a long path or symbol name is split at an
arbitrary character. For code-bearing content that is worse than a horizontal
scrollbar.

**4.3 — A Bash block changes wrap behaviour when it finishes.** While running it
renders through `ChunkList` → `.agent-tool-log-line` (`white-space: pre`, no
wrap). On completion it re-renders through `BashOutputViewer` →
`.agent-terminal-output > div` (`pre-wrap`, wraps). Same content, same panel,
different layout — a visible reflow at the moment the tool completes.
(`ToolOverlayLog.tsx:110-114` selects the branch.)

---

## 5. Two smaller findings in the same area

**5.1 — `.agent-diff` declares no `white-space`, and its element type differs
between the two render paths.** `DiffViewer.tsx:288` renders the plain fallback
as `<pre class="agent-diff">` (UA `white-space: pre`), while `:306` renders the
Shiki path as `<div class="agent-diff agent-diff--highlighted">`
(`white-space: normal`). The code itself is safe either way — on the highlighted
path it sits in an inner `<pre>` (`:308`). But `.agent-diff-header` (the file
path, `:289` and `:307`) is a direct child on both paths, so **a long file path
scrolls on the pre-Shiki render and wraps once Shiki resolves** — a small layout
flip mid-render. It is also a latent trap: any future direct text child of
`.agent-diff` on the highlighted path silently gets `normal` instead of `pre`.

**5.2 — The markdown Read path still feeds line-numbered text to the markdown
renderer.** `ToolOverlayLog.tsx:499-501` passes `dedentedText` — which retains
the `<N>\t` gutter — into `<Markdown>`. Spec §2.1 flagged this in August as a
pre-existing bug and explicitly deferred it; it is still present on main. A line
`1\t# Title` is not a heading, and markdown prose wraps, so a Read of any `.md`
file produces wrapped, mis-rendered output. If the report came from reading a
markdown file, **this alone explains it**.

Related and still undecided: spec §2.1 also parked "whether to keep rendering
the number gutter at all (vs stripping it, vs promoting it to a styled
non-selectable gutter element like the editor has)" as a separate refinement.
It has not been revisited. Note the gutter is currently fed through Shiki as if
it were source, so on a TypeScript preview the line number is lexed and coloured
as a numeric literal.

---

## 6. Recommendations, in priority order

> **Status 2026-09-02:** 6.2–6.6 are implemented on
> `agent4/tool-preview-indent-and-wrap`. 6.1 was **not** done as written — see
> the note under it for what shipped instead and why.

### 6.1 Strip the Read line-number gutter out of the highlighted text (highest value)

Either drop it, or promote it to a real gutter element outside the `<pre>` (a
sibling column, `user-select: none`), as the Editor pane does. This single
change:

- reclaims 2–8 columns of preview width on every Read;
- **eliminates the §2 raggedness entirely**, and makes the `tab-size` value stop
  mattering for the gutter;
- stops line numbers being syntax-highlighted as numeric literals;
- fixes §5.2's markdown corruption for free;
- lets `stripCommonIndentNumbered` collapse back to plain `stripCommonIndent`.

The spec already anticipated this as the natural next refinement. It is the
cheapest fix with the widest blast radius.

> **What shipped instead (2026-09-02).** The *gutter-as-a-separate-column* half
> was not built. A parallel column has to line up with Shiki's line boxes, and
> Shiki emits `<span class="line">…</span>` separated by literal `\n` text
> nodes inside an inline `<code>` — line-box behaviour there is subtle enough
> that I could not verify alignment without a live pane, and a gutter that
> drifts by one line is worse than no change at all.
>
> The *deterministic* half shipped: `renderNumberedGutter` re-emits the number
> **right-aligned to a fixed width, separated by a single space instead of the
> tab**. That gets the parts that mattered — the §2 raggedness is gone (the
> gutter now ends at the same column on every line, unit-tested against a real
> transcript sample), the tab no longer interacts with `tab-size` at all, and
> the markdown path gets a gutter-free body so §5.2's corruption is fixed. What
> it does *not* get: the gutter is still selectable and still passes through
> the syntax highlighter. Those remain a follow-up, and the split/render
> helpers are now factored so that follow-up is a rendering change only.

### 6.2 Normalise indentation *width* in the text, not in CSS

This is the only thing that addresses "a lot of indentation still" for
space-indented files. Detect the preview's indent unit (tab / 2-space /
4-space) from the displayed lines, then re-emit each line's **leading run only**
at a fixed narrow unit — e.g. render every indent level as 2 columns.

Constraints that must hold:

- leading whitespace only; never touch alignment *inside* a line (aligned
  trailing comments, ASCII tables, continuation alignment);
- skip when the unit can't be inferred confidently (mixed/irregular), same
  decline-rather-than-guess posture `dedent.ts` already takes for tabs vs
  spaces;
- skip for whitespace-significant content — Bash output especially, where
  leading spaces may be real table alignment (spec §2 already excludes it);
- apply *after* dedent, so the two compose.

A cheaper first cut: leave the text alone and only apply this to Grep results,
which is where indentation is most useless (each match line is from an unrelated
file position anyway).

### 6.3 Pick one wrapping policy for code-bearing surfaces

Recommendation: **`white-space: pre` + horizontal scroll everywhere code is
shown**, matching what Read/Write/Edit already do. Wrapping code at column 0
destroys exactly the indentation structure that dedent exists to preserve, so
wrapping and dedent are working against each other today.

If wrapping must stay on Bash/Grep output, then at minimum:

- drop `overflow-wrap: anywhere` / `word-break: break-all` from those surfaces
  (`:1445`, `:1773`) so breaks land at token boundaries, not mid-identifier;
- give wrapped lines a hanging indent (`text-indent: -Nch; padding-left: Nch`)
  so continuations align under their own line's indentation instead of
  restarting at column 0.

### 6.4 Fix the streaming→final wrap flip (§4.3)

Make `.agent-tool-log-line` and `.agent-terminal-output > div` agree. Whichever
policy 6.3 picks, both should use it.

### 6.5 Give `.agent-diff` an explicit `white-space` (§5.1)

Declare it on `.agent-diff` itself so both the `<pre>` and `<div>` paths behave
identically and the header stops flipping when Shiki resolves.

### 6.6 Revisit `tab-size: 2`

If 6.1 lands, the gutter conflict disappears and 2 is fine (it then only affects
genuine relative indentation, which is what #2785 wanted). If 6.1 does *not*
land, **`tab-size: 4` is the better value** — it confines gutter raggedness to
the line-1000 boundary instead of the far more common line-10 one. Do not change
this in isolation before deciding 6.1; it is a trade, not a fix.

---

## 7. Confidence and what is not verified

Everything in §1–§5 is read directly from main @ `01cd708e` and cited by
file:line. The tab-stop table in §2 is arithmetic on the CSS `tab-size`
definition.

**Settled by measurement** (real `tool_result` frames, transcript store opened
read-only):

- The `Read` result format is `<N>\t<code>` with the number **left-aligned, not
  padded** — `9\t` then `10\t`. This is what makes §2's raggedness real rather
  than hypothetical. Had the CLI right-aligned the numbers, §2 would not exist,
  and `NUMBERED_LINE_RE` (`dedent.ts:28`) tolerates both shapes, so the code
  alone could not have told us which was live.
- A representative code `Read` is **100% space-indented, 0% tab-indented**
  (0 / 127 lines), with 58 lines at column 0 and 23 at column 20. That
  simultaneously confirms §1 (`tab-size` is inert here) and §3 (the column-0
  lines force dedent to a no-op on the exact preview that most needs it).

**One thing still open, and it changes the priority order:** which surface the
reporter was actually looking at. "Some lines are word-wrapped" is *impossible*
on the Read/Write/Edit code path (`white-space: pre`), so that block was a Bash
output, a Grep result, an expanded JSON blob, an Agent/Task result, or a `.md`
Read. If it was one of the first four, §6.3/§6.4 is the fix; if it was a `.md`
Read, §5.2/§6.1 is. A screenshot settles it in seconds.

The indentation half of the report does not depend on that question — §1, §2 and
§3 hold regardless.

## 8. Reproducing the measurements

```bash
python - <<'PY'
import sqlite3, re, json, collections
con = sqlite3.connect(
    "file:%s/.agentmux/shared/agents/transcripts/filestore.db?mode=ro" % __import__("os").path.expanduser("~"),
    uri=True)
# transcripts are 64 KB parts of a stream-json NDJSON log, keyed by agent zone
z = 'agent:<uuid>:current'
parts = [r[0].decode('utf-8','replace') for r in con.execute(
    "SELECT data FROM db_file_data WHERE zoneid=? AND name='output' "
    "AND partidx<20 ORDER BY partidx", (z,))]
t = "".join(parts)
# find a Read tool_result and decode its JSON string body
m = re.search(r'"type":"tool_result","content":"', t)
...  # scan forward to the closing quote, json.loads, then split on \n
PY
```

Full scripts are in the PR discussion; the two numbers that matter are the
`<N>\t` shape (§2) and the tab-vs-space histogram (§1).
