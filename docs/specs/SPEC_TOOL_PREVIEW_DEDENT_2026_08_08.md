# SPEC: Tool preview common-indentation stripping (dedent)

**Date:** 2026-08-08
**Status:** proposed — verified unimplemented as of 2026-08-10 (sibling refinement #2467 shipped; this one did not).
**Scope:** `frontend/app/view/agent/components/ToolOverlayLog.tsx`,
`frontend/app/view/agent/components/DiffViewer.tsx`, one new shared util
(+ tests)
**Related:** `SPEC_TOOL_PREVIEW_SCROLLBAR_EDGE_PADDING_2026_08_08.md`
(refinement #1 of this series — this is refinement #2)

---

## 1. Report

When a tool preview shows a snippet from the middle of a file — a `Read`
with `offset`, or an `Edit` whose `old_string`/`new_string` sit deep inside
a nested scope — every displayed line carries the file's original leading
indentation. A hunk whose shallowest line is, say, three scopes deep wastes
12-24 columns of the preview on indentation that carries no information
*within the preview*: the reader can't see the enclosing scopes anyway, and
the preview's usable width (already at a premium; see refinement #1) is
spent on empty space.

Wanted: strip the indentation that is common to every line in the preview,
so the **shallowest line renders flush-left** and only the *relative*
indentation between displayed lines — the part that actually encodes
structure — is kept.

## 2. Where preview content comes from (traced, not guessed)

All final-result rendering happens in the per-tool renderers of
`ToolOverlayLog.tsx` (registered via `tool-renderers/registry.ts`):

| Tool | Renderer | Content source | Indentation reality |
|---|---|---|---|
| Read | `renderRead` (ToolOverlayLog.tsx:450) → `HighlightedCode` or `Markdown` | `result.content` — passed through raw from the CLI's `tool_result` | **Line-number-prefixed**: verified from a real session transcript, each line is `N\t<raw line>` (e.g. `80\t        unsetEnv: [...]`). The code after the tab keeps full original indentation. Mid-file reads (offset) are the worst case. |
| Edit | `renderEdit` (:442) → `DiffViewer` | `params.old_string`/`new_string`; `result.diff` "is always undefined in the current pipeline" (DiffViewer.tsx:42-44) — the diff is built client-side via LCS (`buildDiffFromParams`, :83) | Both strings are raw mid-file snippets sharing the target's deep indentation. |
| Write | `renderWrite` (:499) → `HighlightedCode` or `Markdown` | `params.content` — the full file being written | Whole files start at column 0; common indent is ~always 0. Dedent is a natural no-op. |
| Bash | `renderBash` → `BashOutputViewer` | stdout/stderr | **Out of scope** — command output has no "file indentation" semantic; leading whitespace can be meaningful (e.g. aligned table output). |
| Grep/Glob | `renderSearch` → `CompactResult` | independent match lines from arbitrary file positions | **Out of scope** for this pass — each match line is independent, so "common" indent across unrelated lines is semantically weak. Candidate follow-up: per-line leading-whitespace trim with an ellipsis affordance, decided separately. |
| Streaming chunks (`ChunkList`) | any running tool | incremental chunks | **Out of scope** — dedent needs the full visible text to know the common prefix; applying it per-chunk mid-stream would shift alignment as lines arrive. Final-result rendering replaces the chunk view on completion, which is where dedent applies. |

No dedent/strip-indent utility exists anywhere in `frontend/` today
(searched).

### 2.1 The Read line-number prefix — a forcing constraint

Because `result.content` lines are `N\t<code>`, a naive
common-prefix-of-the-whole-line dedent would find no common whitespace (the
line starts with a digit) and do nothing. The dedent must therefore split
each Read line into `(number-prefix, code)` and dedent the code portion.

**Implementer must first verify in a live pane what the current Read
preview actually renders** (one Read of a mid-file section in `task dev`):
the working assumption from the transcript sample is that the `N\t` prefix
is rendered verbatim today (Shiki highlights it as part of the code). Two
consequences if confirmed:

- The markdown Read path (`.md`/`.mdx` → `<Markdown text={capped.text}/>`,
  ToolOverlayLog.tsx:481-483) is feeding *numbered* lines into a markdown
  renderer — that's a pre-existing rendering bug of its own (a `1\t# Title`
  line is not a heading). Fixing that fully is its own change; this spec
  only requires that dedent not make it *worse* (see §3.3).
- Whether to keep rendering the number gutter at all (vs stripping it, vs
  promoting it to a styled non-selectable gutter element like the editor
  has) is a **separate refinement** — deliberately not decided here. This
  spec's algorithm preserves whatever prefix policy is in place: it
  dedents the code portion and leaves the prefix handling unchanged.

## 3. Design

### 3.1 New shared utility — `dedent.ts`

`frontend/app/view/agent/components/dedent.ts`, two pure functions, unit
tested:

```ts
/** Longest common leading-whitespace prefix across all non-blank lines.
 *  Compared LITERALLY (string prefix), so tabs vs spaces never get
 *  conflated via an assumed tab width: "\t\tfoo" and "    foo" share no
 *  prefix and the dedent is a no-op — correct, since we can't know how
 *  wide a tab renders. Blank / whitespace-only lines are ignored for the
 *  computation and emptied in the output. */
export function stripCommonIndent(text: string): string;

/** Read-tool variant: if every non-blank line matches /^\s*\d+\t/, split
 *  off that prefix, apply stripCommonIndent to the code portions, and
 *  rejoin prefix + code. Otherwise falls through to stripCommonIndent on
 *  the whole text. */
export function stripCommonIndentNumbered(text: string): string;
```

Algorithm for `stripCommonIndent` (single pass over lines):

1. Split on `\n`. Collect the leading-whitespace run (`/^[ \t]*/`) of each
   non-blank line; the common prefix is the shortest such run that is a
   literal prefix of all the others (equivalently: fold with
   longest-common-prefix, whitespace-only by construction).
2. If the common prefix is empty → return the input unchanged (fast path;
   covers Write and already-flush content with zero allocation churn).
3. Otherwise remove that exact prefix from every non-blank line; blank
   lines pass through as-is.

### 3.2 Application points

1. **Read** (`renderRead`): `capped.text` →
   `stripCommonIndentNumbered(capped.text)` — **after** `capText`, so the
   common indent is computed over the *visible* lines only (a deeper hidden
   tail must not reduce the dedent of what's shown; head-cap keeps the
   first `MAX_TOOL_OUTPUT_LINES` lines, ToolOverlayLog.tsx:456).
   Applies to both the Shiki path and the markdown path.
   - `detectLanguage(filePath, firstLine)` (:476) keeps receiving the
     original first line — language detection by shebang/content should
     see the raw text; only the *displayed* string is dedented.
2. **Edit** (`DiffViewer`): dedent `old_string` and `new_string` with **one
   shared prefix** (compute over the concatenation of both strings' lines,
   then strip from each) *before* `buildDiffFromParams` — a shared prefix
   preserves add/del alignment; independent dedents could skew one side by
   a level and manufacture phantom diff noise. The `result.diff`-present
   branch (currently dead per the file's own comment) passes through
   untouched.
3. **Write** (`renderWrite`): same call as Read (numbered variant is
   harmless — Write content has no number prefixes, so it falls through to
   the plain path, which is a no-op for column-0 files). Included for
   uniformity so a hypothetical indented Write (e.g. a snippet-shaped
   file) behaves consistently.

### 3.3 What dedent must NOT do

- **No per-line trimming.** Only the *common* prefix is removed; relative
  indentation is sacred — that's the "only retain what is necessary" half
  of the requirement.
- **No tab-width assumptions.** Literal prefix comparison only (§3.1). A
  file mixing tabs and spaces at the same depth dedents by whatever prefix
  is genuinely shared, possibly nothing. Correct > clever.
- **No markdown-path regression.** For `.md` Reads, dedent runs on the
  code portion after number-prefix splitting, same as the Shiki path. If
  the live-pane check (§2.1) reveals the markdown path renders numbered
  lines today, file the gutter/prefix question as its own follow-up rather
  than expanding this change's blast radius.
- **No streaming-path changes.** `ChunkList` and `BashOutputViewer` are
  untouched.

## 4. Risks

1. **Copy behavior changes**: selecting + copying from a dedented preview
   yields dedented text. For a preview (not an editor surface) this is the
   expected reading-oriented trade-off; the file itself is one click away
   via the path header. Called out so it's a decision, not an accident.
2. **Shiki cache**: `HighlightedCode` caches per-node via WeakMap keyed on
   the node — the dedented string is stable per render input, so caching
   is unaffected.
3. **Diff correctness**: the shared-prefix rule (§3.2.2) is the one place
   dedent could corrupt meaning if done per-side; the unit tests must
   cover an old/new pair whose sides have *different* minimum depths
   (e.g. new_string adds a shallower wrapper line) and assert the shared
   prefix is the min of the two.
4. **Pathological inputs**: single-line content, all-blank content, empty
   string, CRLF line endings (`\r` must not be treated as part of the
   indent — normalize by treating `\r?$` as line end, or strip on `\n` and
   tolerate a trailing `\r` in the whitespace regex). All unit-test cases.

## 5. Test plan

Unit (vitest, `dedent.test.ts`):
- [ ] Uniform space indent stripped to flush; relative levels preserved.
- [ ] Tab-indented content stripped by literal tab prefix.
- [ ] Mixed tab/space with no true common prefix → unchanged.
- [ ] Blank lines ignored for computation, emptied in output.
- [ ] Numbered (`N\t`) Read-shaped input: prefix preserved, code dedented.
- [ ] Non-uniformly-numbered input falls through to plain dedent.
- [ ] Shared-prefix diff dedent with asymmetric old/new depths.
- [ ] Empty / single-line / CRLF inputs.

Visual (per the live-pane check that's a precondition anyway):
- [ ] Read of a mid-file deeply-nested section: shallowest line flush,
      structure preserved, horizontal scrollbar appears less often.
- [ ] Edit of a deeply-nested hunk: add/del lines aligned, no phantom
      indentation diffs.
- [ ] Write of a normal file: pixel-identical to before (no-op path).
- [ ] Read of a `.md` file: not worse than current rendering.

## 6. Summary

One pure utility (literal longest-common-whitespace-prefix stripping, with
a numbered-line variant for Read's verified `N\t` content shape), applied
at three call sites after capping: Read and Write displays dedent their
visible text, and DiffViewer dedents `old_string`/`new_string` with a
single shared prefix before building its LCS diff. Bash, Grep/Glob, and
streaming chunks are explicitly out of scope. The pre-existing
numbered-lines-through-Markdown oddity and the "should the number gutter
exist at all" question are surfaced but deferred to their own refinement.
