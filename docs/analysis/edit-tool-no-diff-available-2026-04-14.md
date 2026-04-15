# Report: Why "No diff available" always shows on Edit tool expansion

**Date:** 2026-04-14
**Severity:** Visible bug — every Edit tool expansion shows an error stub instead of the actual edit
**Affected:** `frontend/app/view/agent/components/DiffViewer.tsx`,
`frontend/app/view/agent/stream-parser.ts`, Claude provider translator

---

## Symptom

Every `Edit` tool block in the agent pane, when hovered/pinned open,
shows:

```
No diff available
File: /path/to/file
```

It's never anything else. Success, failed, running — always this stub.

## Root cause

It's a **dead shape bug**: the `EditResult` type exists in
`types.ts:120` but nothing in the codebase ever constructs one.

### Trace through the code

1. **`frontend/app/view/agent/types.ts:120`** declares:
   ```ts
   export interface EditResult {
       linesChanged: number;
       diff?: string;
   }
   ```

2. **`frontend/app/view/agent/components/DiffViewer.tsx:17`** reads:
   ```tsx
   const diff = result?.diff;
   if (!diff) {
       return (
           <pre class="agent-diff-empty">
               No diff available
               {"\n"}
               File: {params.file_path}
           </pre>
       );
   }
   ```

3. **`frontend/app/view/agent/stream-parser.ts:207-234`** populates the
   tool node:
   ```ts
   return {
       type: "tool",
       id: event.id,
       tool: this.normalizeToolName(toolName),
       params,
       status: event.status,
       duration: event.duration,
       result: event.result,   // ← straight from the translator
       ...
   };
   ```

4. **`frontend/app/view/agent/providers/claude-translator.ts:179-188`**
   takes Claude's `tool_result` block and passes its content through
   as-is. Claude's Edit tool API response is a plain-text string:

   ```
   The file /path/to/file.ts has been updated. Here's the result of
   running `cat -n` on a snippet of the edited file:
      1  import { foo } from "bar";
      2  ...
   ```

   — which gets stored as a string. The translator does not construct
   a JS object with `{diff: "..."}`, and nothing downstream of it
   does either.

5. Therefore, when `DiffViewer` does `result?.diff`, it hits one of
   two cases depending on what `result` actually is at runtime:

   - **Case A:** `result` is a string (Claude's Edit response text).
     `"some string".diff` is `undefined`. Fall into the empty state.
   - **Case B:** `result` is an object (e.g. from some error path or
     the compact result shape). No `.diff` key exists. Same result.

Either way: "No diff available" shows, every single time. The
`EditResult` interface is a fossil — someone wrote the type expecting
the backend to populate it, then either the backend work never
happened or the translator never hooked it up.

### Why this wasn't caught earlier

- The string branch of `result` happens to work for `Read` (which
  reads `result.content`) and `Bash` (reads `result.output`) because
  the Claude translator for those builds the expected shape. For
  `Edit`, nobody built the shape.
- There's no `grep -r '\.diff\s*=' frontend/app/view/agent/` hit.
- There's no translator test asserting that an Edit tool_result
  produces an object with a `diff` field.
- The one-line collapsed-by-default tool UI (PR #367 /
  `SPEC_TOOL_OVERLAY_AND_SCROLL_ON_TYPE_2026_04_13.md`) means most
  users hover only occasionally and it reads as "huh, weird" rather
  than "this is broken."

---

## Important observation: we don't need Claude to send a diff

The `EditParams` we already have on every Edit tool call contains:

```ts
export interface EditParams {
    file_path: string;
    old_string: string;
    new_string: string;
    replace_all?: boolean;
}
```

That is literally enough to render a meaningful diff in the UI,
**without any backend work and without any library**. Claude's Edit
tool is `old_string → new_string`; the viewer just has to show both.

## Fix options, ranked

### Option A — Render from params (recommended, ~1h)

Change `DiffViewer` to ignore `result.diff` entirely and render from
`params.old_string` / `params.new_string`:

```tsx
export const DiffViewer = ({ params, result }: DiffViewerProps): JSX.Element => {
    const oldLines = params.old_string.split("\n");
    const newLines = params.new_string.split("\n");

    return (
        <pre class="agent-diff">
            <div class="agent-diff-header">{params.file_path}</div>
            <For each={oldLines}>
                {(line) => <div class="agent-diff-del">-{line}</div>}
            </For>
            <For each={newLines}>
                {(line) => <div class="agent-diff-add">+{line}</div>}
            </For>
            <Show when={result && (result as any).error}>
                <div class="agent-diff-error">
                    {(result as any).error}
                </div>
            </Show>
        </pre>
    );
};
```

**Pros:**
- Zero new dependencies.
- Works for every Edit, always.
- The "diff" matches exactly what the model asked for (it IS the
  old/new string pair).
- Status is surfaced by the surrounding `agent-tool-block` status
  class, so failures still show a red ✗ on the collapsed row.

**Cons:**
- Not a unified diff — `old_string` and `new_string` are shown
  independently, so if a 50-line replacement has 45 unchanged lines
  you see 45 deletions + 45 additions. Rare in practice since Claude
  picks minimal old_strings, but noted.

### Option B — Compute a real unified diff client-side (~2h)

Add the `diff` npm package (20 KB) and call `diffLines(old, new)` to
produce a proper unified diff with context lines. Render the result
through the existing diff line classes.

**Pros:**
- Looks exactly like `git diff`.
- Collapses unchanged lines correctly.

**Cons:**
- New dependency.
- More code for a case that's rarely hit by long replacements.
- Still doesn't show surrounding file context — we don't have the
  full file on hand.

### Option C — Populate `EditResult.diff` from the backend (not recommended)

Plumb a real unified diff through the Claude translator. Requires
writing a git-diff-compatible diff in the backend, passing it
through the WebSocket, and updating the translator to split the
Claude string response from our structured result.

**Pros:**
- Matches the original type design.

**Cons:**
- Lot of wiring for no visible improvement over Option A.
- Adds a code path that only works when the backend is AgentMux's
  own — any provider that doesn't speak our protocol still sees
  "No diff available."
- Claude's API never returns a structured diff, so the backend
  would have to compute it locally anyway, duplicating Option B's
  work.

### Recommendation

**Go with Option A.** It's the smallest change, uses data we already
have, and closes the visible bug. Option B is a nice follow-up if
someone complains about large replacements looking weird, and the
`diff` package is trivial to drop in later.

After Option A lands, **delete the `diff?: string` field from
`EditResult`** since nothing writes it. Also delete the
`if (!diff) { return <No diff available> }` branch entirely.

---

## Interaction with the code-highlighting spec

`SPEC_TOOL_OVERLAY_CODE_HIGHLIGHTING_2026_04_14.md` step 3 is
"DiffViewer with highlighted diff lines." That spec assumes
DiffViewer actually renders lines — it currently doesn't.

**Order of operations:**
1. Fix DiffViewer to render from params (this report, Option A).
2. *Then* land the highlighting spec step 3 on top — it's a small
   transformer over the same `For each={lines}` loop.

Landing highlighting first on a viewer that never renders anything
would be a no-op user-visibly.

---

## Related dead shapes to audit

Since `EditResult.diff` was never populated, worth a quick check
for siblings:

- **`ReadResult.lines`** (types.ts:117) — is this ever set? Used?
- **`WriteResult.bytesWritten`** (types.ts:125) — see
  `ToolBlock.tsx:228` which reads `(result as any).bytesWritten`. If
  the translator passes Claude's plain-text response, this is `undefined`
  and the UI shows `Wrote undefined bytes` or falls through. **Probably
  the same bug in miniature.**
- **`BashResult.output`** — verify the translator actually sets this.
  `BashOutputViewer` depends on it being populated.

These should each get a 5-line grep to confirm. If any are also dead,
bundle the fixes with the DiffViewer one so the whole "tool result
shapes" layer gets a consistency pass.

---

## Suggested PR scope

Single PR titled **"fix(agent): render Edit tool diff from params"**
with:

1. `DiffViewer.tsx` → render from `params.old_string` / `params.new_string`
2. `types.ts` → remove `diff?: string` from `EditResult`; keep
   `linesChanged` for now (it's also dead, but not visible)
3. `ToolBlock.tsx` → unchanged (already passes params + result)
4. Smoke test: open an agent pane, run an Edit, hover the tool block,
   verify the old/new strings appear instead of the stub.

No backend changes. No new deps. Probably ~40 lines of delta.

---

## Appendix: reproduction

1. Launch AgentMux, open an agent pane running Claude.
2. Ask Claude to edit any file: `edit foo.ts to add a comment on line 1`.
3. Wait for the Edit tool block to appear in the document.
4. Hover it (or click to pin).
5. Observe: `No diff available\nFile: foo.ts` — the stub, every time.

Expected (post-fix): `-<old_string>\n+<new_string>` with the existing
red/green diff chrome.
