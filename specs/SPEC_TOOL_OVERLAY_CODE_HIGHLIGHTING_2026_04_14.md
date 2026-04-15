# Spec: Syntax highlighting inside the tool hover overlay

**Status:** Draft
**Date:** 2026-04-14
**Scope:** `frontend/app/view/agent/components/ToolBlock.tsx` + children

---

## 1. Why this matters

Tool blocks in the agent pane are one-line collapsed by default (per
`docs/specs/tool-collapse.md` + SPEC_AGENT_PANE_FOLLOWUPS items #4/#5).
Hover expands them in a portal overlay that shows the full tool content
— file reads, diffs, bash output, etc.

Right now that overlay renders code as plaintext:

- `Read` → raw `<pre>` of the file contents
- `Edit` → `DiffViewer` splits by `+` / `-` / `@` but each line is still
  plain characters; no keyword / identifier / string coloring
- `Write` → currently only shows bytes-written, so no code is visible
- `Bash` → command string and output are plaintext
- `Grep` results → via `CompactResult`, plaintext

Scrolling back through a long session to find "the place where I edited
the dispatcher" is visually jarring because every tool expansion looks
like a wall of monospace. The user can't scan diffs the way they can in
the rest of the app (the markdown panel uses highlight.js and Streamdown
uses Shiki — both produce colored output).

A ~30 minute addition using the already-bundled Shiki pipeline closes
that gap: `Read` file contents, `Edit` diff bodies, and `Bash` commands
all show up in the same GitHub-dark-high-contrast theme already in use
elsewhere in the app, with zero new dependencies.

---

## 2. Current state

- `ToolBlock.tsx` branches on `node.tool` and delegates to:
  - `DiffViewer` (for Edit)
  - `BashOutputViewer` (for Bash)
  - Inline `<pre>` with `{(node.result as any).content}` (for Read)
  - `CompactResult` (Grep/Glob/Agent/Task/default)
- **No syntax highlighting anywhere.** Each component renders its text
  directly into a `<pre>` or `<div>`.

**Shiki is already in the repo** (`shiki@^3.21.0` in package.json) and
lazy-loaded in `frontend/app/element/streamdown.tsx`:

```ts
let shikiModule: typeof import("shiki/bundle/web") | null = null;
const getShiki = async () => {
    if (!shikiModule) {
        shikiModule = await import("shiki/bundle/web");
    }
    return shikiModule;
};
```

The theme constant in that file is `"github-dark-high-contrast"`. The
same theme should be reused here for visual consistency.

**highlight.js is also bundled** (used by `markdown.tsx` with
`github-dark-dimmed.scss`), but Shiki is the right pick here because
(a) Streamdown already pulled it into the chunk we pay for, so lazy
loading is essentially free on the second open, and (b) Shiki uses
real TextMate grammars so diff + code look right.

---

## 3. Target

When the user hovers (or pins) a tool block, the overlay body renders
code with the same visual quality as Streamdown: keyword coloring,
string literals, comments, numbers. Specifically:

- **`Read`**: file contents are highlighted in the language implied
  by the file extension. File path header stays plain.
- **`Edit`**: diff body is highlighted in the file's language, with
  the existing `agent-diff-add` / `-del` / `-hunk` / `-ctx` line
  backgrounds layered *behind* the token coloring (so you still see
  green/red for added/removed lines, but the code inside those lines
  is colored normally). File path header stays plain.
- **`Write`**: currently a bytes-written stub — out of scope here, but
  the spec leaves room to show the written content when
  `(result as any).content` is present.
- **`Bash`**: the command (`params.command`) is highlighted as `bash`;
  the output is plaintext (it's rarely source code, and guessing is
  worse than plain).
- **`Grep` / `Glob` / `Agent` / `Task` / default**: unchanged for
  v1 — these go through `CompactResult` which isn't meaningfully
  code. Spec leaves this for a follow-up if we add a `Read` fallback
  for Grep match context.

---

## 4. Design

### 4.1 Component shape

New file: `frontend/app/view/agent/components/HighlightedCode.tsx`

```ts
interface HighlightedCodeProps {
    code: string;
    /**
     * Shiki language id (e.g. "typescript", "python", "bash"). Falls
     * back to "text" when unknown or when detection returns null.
     */
    lang: string;
    /**
     * Optional extra class for the rendered <pre>. Used by callers
     * like DiffViewer to add .agent-diff or similar.
     */
    class?: string;
}
```

Internally:

1. Mounts with a plain `<pre>` showing unhighlighted text as the
   placeholder — same perf posture as streamdown.tsx, so the first
   paint is never blocked on the Shiki chunk.
2. `createEffect` kicks off `getShiki()` and calls
   `highlighter.codeToHtml(code, { lang, theme: "github-dark-high-contrast" })`.
3. On completion, swaps the placeholder for the Shiki-generated HTML
   via `innerHTML`. This is safe — Shiki output is sanitized HTML
   wrapping `<span>` elements.
4. **Seq guard** like streamdown.tsx: if props change before the
   highlight completes, drop the stale result. Same `seqRef++` pattern.
5. **Size cap**: if `code.length > CAP_BYTES` (default 200 KB) or
   `line_count > CAP_LINES` (default 2000), skip highlighting entirely
   and render plaintext. Huge files would stall the main thread
   otherwise.
6. **Error path**: if `getShiki()` throws (rare — usually a network
   hiccup on first load) render plaintext and don't retry. Log via
   `console.warn` for observability.

### 4.2 Language detection

New helper: `frontend/app/view/agent/components/detectLanguage.ts`

```ts
/**
 * Map a file path (absolute or relative) to a Shiki language id.
 * Returns "text" for unknown extensions so the caller can still
 * route through HighlightedCode without branching.
 */
export function detectLanguage(filePath: string): string;
```

Detection strategy (first hit wins):

1. **Extension map** — a static Record<string, string> covering at
   least: `ts tsx js jsx mjs cjs` → typescript/tsx, `py` → python,
   `rs` → rust, `go` → go, `sh bash zsh` → bash, `ps1` → powershell,
   `md mdx` → markdown, `json` → json, `yaml yml` → yaml,
   `toml` → toml, `css scss sass` → scss, `html htm` → html,
   `sql` → sql, `xml` → xml, `rb` → ruby, `java` → java,
   `kt` → kotlin, `swift` → swift, `c h` → c, `cpp hpp cxx hxx` →
   cpp, `cs` → csharp, `php` → php, `lua` → lua, `vue` → vue,
   `svelte` → svelte, `dockerfile` → dockerfile, `tf` → terraform,
   `graphql gql` → graphql.
2. **Filename match** for things without extensions: `Dockerfile`,
   `Makefile`, `.gitignore` → ignore, `.env*` → bash.
3. **Shebang scan** of the first line for files without a recognized
   extension: `#!/usr/bin/env python3` → python, `#!/bin/bash` → bash,
   etc. Only scans if the first line starts with `#!`.
4. **Fallback** → `"text"` (Shiki's plain-text path, no grammar load).

Detection is pure + synchronous. The handful of cases it gets wrong
are acceptable v1 failure modes — this isn't the LSP.

### 4.3 `Read` integration

In `ToolBlock.tsx`, replace:

```tsx
case "Read":
    return (
        <div class="agent-tool-read">
            <div class="agent-tool-file-path">{(node.params as any).file_path}</div>
            <Show when={node.result}>
                {(node.result as any).content ? (
                    <pre class="agent-tool-read-content">{(node.result as any).content}</pre>
                ) : (
                    <CompactResult .../>
                )}
            </Show>
        </div>
    );
```

with:

```tsx
case "Read": {
    const filePath = (node.params as any).file_path;
    const content = (node.result as any)?.content;
    return (
        <div class="agent-tool-read">
            <div class="agent-tool-file-path">{filePath}</div>
            <Show when={content} fallback={<CompactResult .../>}>
                <HighlightedCode
                    code={content}
                    lang={detectLanguage(filePath)}
                    class="agent-tool-read-content"
                />
            </Show>
        </div>
    );
}
```

### 4.4 `Edit` / `DiffViewer` integration

Two viable approaches:

**A. Per-line highlight with diff chrome overlay.** Iterate the diff
lines; strip the leading `+`/`-`/` ` marker; pass each *body* through
the highlighter with the file's language; wrap the result in the
existing `.agent-diff-add` / `-del` / `-ctx` class. Hunk headers
(`@@ -1,5 +1,5 @@`) stay plain.

Pro: tokens inside added/removed lines are colored correctly; the
green/red line background is just a CSS class on the wrapping div.
Con: calling `codeToHtml` 200 times for a 200-line diff is wasteful.

**B. Highlight the whole diff body once as the file's language.**
Pass the entire diff (with `+`/`-` markers) to
`codeToHtml(..., { lang: "diff" })` — Shiki has a built-in `diff`
grammar that colors add/remove/hunk markers. This gets you line-level
coloring but loses token-level coloring inside added/removed code.

**Recommended: hybrid.** Call
`codeToHtml(diffBody, { lang: detectLanguage(filePath), transformers: [diffTransformer] })`
where `diffTransformer` is a tiny Shiki transformer (supported first-
class in Shiki 3.x) that:

1. Detects lines starting with `+`/`-`/` ` in the source.
2. Strips the marker before highlighting (so the grammar sees valid
   code, not `+const foo = 1;`).
3. Adds the `diff-add` / `diff-del` / `diff-ctx` class to the
   generated `<span class="line">` wrapper.
4. Re-inserts the marker as a leading non-highlighted `<span>`.

Shiki's `transformerNotationDiff` package does almost exactly this but
expects `// [!code ++]` annotations, not raw `+`/`-` prefixes — so
we'll write a ~30-line custom transformer inline in DiffViewer. The
existing `.agent-diff-add` / `-del` / `-hunk` / `-ctx` SCSS rules stay
in place but apply to the `.line` elements Shiki emits.

DiffViewer's current empty-state path (`No diff available`) stays
plaintext — out of scope.

### 4.5 `Bash` integration

`BashOutputViewer` currently renders `params.command` and
`result.output` separately. Update it to:

- **Command**: `HighlightedCode` with `lang="bash"`.
- **Output**: stays as plaintext `<pre>`. Guessing at output format
  (JSON? Python? log?) is worse than plain. If the command was
  something like `cat foo.json`, the user can still see structure via
  indentation.

### 4.6 Theme

Reuse the existing `"github-dark-high-contrast"` constant already
picked in `streamdown.tsx`. Export it from a new
`frontend/app/view/agent/commands/theme.ts` … actually just hardcode
it in `HighlightedCode.tsx` with a `// TODO: read from settings` comment.
Theme switching isn't in the spec and AgentMux is dark-theme-only right
now anyway.

### 4.7 Caching

Highlighting is pure: `(code, lang, theme) → html`. The same tool call
keeps its code forever (ToolNode is immutable), so:

- Use a per-node cache keyed by `node.id` — a `WeakMap<ToolNode, string>`
  at module scope in `HighlightedCode.tsx`.
- On first render, check the cache; if present, render directly without
  round-tripping through Shiki.
- On first render without cache, kick off the async highlight; on
  completion, store the HTML in the cache before swapping it in.

This matters because hovering the same tool block multiple times is
the expected UX — the user hovers, reads, scrolls, hovers again. Each
re-hover should be instant.

### 4.8 Performance guardrails

- **Size cap** as noted: `> 200 KB` OR `> 2000 lines` → skip highlight,
  render plaintext. Log once via `console.debug` so we can spot cases
  where the cap trips in practice.
- **Lazy grammar load**: Shiki's `bundle/web` ships every grammar we
  need in one chunk. Already paid for by Streamdown. Second use is
  free.
- **Main-thread highlighting is fine for the sizes we care about.**
  Shiki doesn't offload to a worker in the web bundle and running a
  TextMate grammar over 2000 lines is sub-100ms on the dev machine.
  If that changes, Shiki exposes a `createHighlighterCore` API that
  could be moved to a worker — marked as a follow-up.

---

## 5. Out of scope

- Light theme support (AgentMux is dark-only for now)
- Language auto-detection from content (no extension + no shebang ⇒ plaintext)
- Grep match highlighting — `CompactResult` doesn't render full file
  context, so there's nothing code-like to color
- Re-highlighting markdown panel or Streamdown — they already highlight
- Theme picker or per-pane theme override
- Worker-offloaded highlighting

---

## 6. Implementation steps

Each step is self-contained and testable.

### Step 1 — `detectLanguage.ts` + unit test

- Create the helper with the extension/filename/shebang logic.
- Add a small test file at
  `frontend/app/view/agent/components/detectLanguage.test.ts` that
  asserts: `foo.ts → typescript`, `Dockerfile → dockerfile`,
  `foo.unknown → text`, `#!/usr/bin/env python3 → python`.
- No behavior change to the app — can ship as its own PR.

### Step 2 — `HighlightedCode.tsx`

- Component with the structure above: placeholder → async Shiki
  swap → WeakMap cache.
- Handles size cap + error fallback.
- Drop-in replacement for `<pre class="agent-tool-read-content">`.
- **Wire into `Read` case only** in this step. Leave `Edit` and
  `Bash` alone. Ship, validate, then move on.

### Step 3 — `DiffViewer` with highlighted diff lines

- Custom Shiki transformer that strips `+`/`-` prefix, highlights
  the underlying code, re-applies the diff class on the `.line`
  wrapper.
- Fallback to the existing plain-diff rendering when
  `getShiki()` rejects.
- Keep the `No diff available` path unchanged (see separate report on
  when/why that path trips — it may become real work).

### Step 4 — `BashOutputViewer` command highlighting

- `HighlightedCode` for `params.command` with `lang="bash"`.
- Output stays plaintext.
- ~10 lines of change.

### Step 5 — SCSS polish

- Shiki emits its own `<span class="line">` wrappers. Ensure our
  `.agent-tool-read-content`, `.agent-diff`, and `.agent-diff-*`
  classes compose cleanly with those. Mostly removing redundant
  `color:` rules so Shiki's tokens win.
- Check font-family: Shiki's generated `<pre>` has no default; we
  want `var(--monospace-font)`.

---

## 7. Success criteria

After Steps 1-5 land:

- `Read` overlay of a TypeScript file shows keyword / string / comment
  coloring matching the rest of the app.
- `Edit` overlay of the same file shows diff chrome (green/red line
  backgrounds) with token-level coloring inside each line.
- `Bash` overlay shows the command in bash highlighting; output plain.
- No observable latency on hover: first hover may flash plaintext for
  ~50ms (the Shiki chunk load); subsequent hovers for the same node
  are instant thanks to the WeakMap cache.
- Unknown extensions render as plaintext without crashing.
- Files > 200 KB render as plaintext without stalling.
- No new dependencies in `package.json`.

---

## 8. Estimated cost

| Step | Time | Risk |
|---|---:|---|
| 1. detectLanguage + test | 45 min | Low |
| 2. HighlightedCode + Read integration | 1h | Low — same pattern as streamdown.tsx |
| 3. DiffViewer transformer | 1.5h | Medium — custom Shiki transformer |
| 4. Bash command highlight | 20 min | Low |
| 5. SCSS polish | 30 min | Low |

**Total: ~4 hours.** Each step is independent, so they can ship as
separate PRs.

---

## 9. Open questions

1. Should the size cap be a setting? Leaning no — 200 KB / 2000 lines
   is a reasonable default and nobody will tweak it. If someone does
   want to, they can open a follow-up.
2. Should we lift `HighlightedCode` to a shared component in
   `frontend/app/element/` so markdown / streamdown / tool overlay all
   converge on one implementation? Leaning yes *after* this lands —
   ship it scoped first, then refactor upward in a separate PR once
   we're sure the API is right.
3. Shiki's `diff` grammar vs. custom transformer: I prefer the custom
   transformer for token-level coloring. Confirm by eyeballing before
   committing to the approach.
