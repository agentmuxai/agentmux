# SPEC: Write tool expanded content view

**Date:** 2026-06-19
**Status:** Planned (implemented — see note below)
**Author:** smike

> **2026-08-07 audit note:** Implemented (`renderWrite()` in
> `ToolOverlayLog.tsx`, PR #1601 — also confirmed by the extending spec
> below citing it as implemented). Status field was never updated. See
> `docs/reports/REPORT_DOCS_AND_DEAD_CODE_CLEANUP_AUDIT_2026_08_07.md`.
**Related:** `frontend/app/view/agent/components/ToolOverlayLog.tsx` (`renderWrite`),
             `frontend/app/view/agent/components/HighlightedCode.tsx`,
             `frontend/app/view/agent/types.ts` (`WriteParams`, `WriteResult`)

---

## 1. Problem

The Write tool's expanded overlay shows only the file path and a byte count:

```
frontend/app/foo.ts
Wrote 1024 bytes
```

The content that was written is already available at `node.params.content` (it's
the tool input, always present) but is not displayed. The agent wrote it; the user
can't see what without opening the file.

---

## 2. Current implementation

**`renderWrite` in `ToolOverlayLog.tsx` (lines 287–296):**

```typescript
function renderWrite(node: ToolNode): JSX.Element {
    return (
        <div class="agent-tool-write">
            <div class="agent-tool-file-path">{(node.params as any).file_path}</div>
            <div class="agent-tool-write-info">
                {node.result && `Wrote ${(node.result as any).bytesWritten || 0} bytes`}
            </div>
        </div>
    );
}
```

**Type shapes (`types.ts`):**

```typescript
export interface WriteParams {
    file_path: string;
    content: string;   // ← the written content, always present
}

export interface WriteResult {
    bytesWritten: number;
}
```

The Read renderer (`renderRead`, lines 256–285) already uses `HighlightedCode` with
`detectLanguage` — the exact infrastructure needed here.

---

## 3. Design

### 3.1 Show written content with syntax highlighting

Reuse `HighlightedCode` + `detectLanguage` (same as `renderRead`). The content comes
from `node.params.content`, not `node.result`.

Cap at `MAX_TOOL_OUTPUT_LINES` lines, head-truncated (top of file is most relevant
for written files; consistent with Read capping).

### 3.2 Header: file path + byte count badge

Collapse the path and byte count onto one line:

```
frontend/app/foo.ts   [1 024 B]
```

- File path on the left (existing `.agent-tool-file-path`)
- Byte count badge on the right, muted, only when `result.bytesWritten` is set
- Format: `1 024 B` / `12.3 KB` / `1.2 MB` (human-readable, narrow)

### 3.3 Updated renderWrite

```typescript
function renderWrite(node: ToolNode): JSX.Element {
    const filePath = (node.params as any).file_path ?? "";
    const content: string | undefined = (node.params as any).content;
    const bytes: number | undefined = (node.result as any)?.bytesWritten;
    const capped = content ? capText(content, MAX_TOOL_OUTPUT_LINES, "head") : null;
    return (
        <div class="agent-tool-write">
            <div class="agent-tool-file-path-row">
                <span class="agent-tool-file-path">{filePath}</span>
                <Show when={bytes != null}>
                    <span class="agent-tool-write-bytes">{formatBytes(bytes!)}</span>
                </Show>
            </div>
            <Show when={capped} fallback={<div class="agent-tool-write-info">No content</div>}>
                <HighlightedCode
                    code={capped!.text}
                    lang={detectLanguage(filePath, capped!.text.split("\n")[0])}
                    class="agent-tool-write-content"
                />
                <Show when={capped!.hiddenLines > 0}>
                    <OutputHiddenMarker hidden={capped!.hiddenLines} noun="line" from="head" />
                </Show>
            </Show>
        </div>
    );
}
```

`formatBytes` is a small local helper (no new dependency):

```typescript
function formatBytes(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}
```

---

## 4. Files changed

| File | Change |
|------|--------|
| `frontend/app/view/agent/components/ToolOverlayLog.tsx` | Replace `renderWrite` with the version above; add `formatBytes` helper |
| `frontend/app/view/agent/styles/_document-nodes.scss` | Add `.agent-tool-file-path-row`, `.agent-tool-write-bytes`, `.agent-tool-write-content` styles |

No new components needed — `HighlightedCode`, `detectLanguage`, `capText`,
`OutputHiddenMarker` are already imported in `ToolOverlayLog.tsx`.

---

## 5. Acceptance criteria

- Expanded Write overlay shows the written content with syntax highlighting
- Language is auto-detected from the file extension (same rules as Read)
- Content is head-capped at `MAX_TOOL_OUTPUT_LINES`; `OutputHiddenMarker` shown when truncated
- File path and byte count appear on one header row; byte count is omitted when result not yet available
- `npx tsc -p tsconfig.json --noEmit` clean (ignoring pre-existing errors)
- No regression on Read tool rendering (shared infrastructure, different component path)
