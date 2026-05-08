# Spec: Refined Table Formatter for Presentation Layer

**Date:** 2026-05-07  
**Status:** Draft  
**Area:** `frontend/app/element/`

---

## Background

Tables are currently rendered in two places with minimal inline Tailwind classes and no
dedicated component:

- **`streamdown.tsx:259-264`** — inline component map passed to `<WaveStreamdown>`:
  ```tsx
  table: (tProps: any) => <table {...tProps} class="w-full border-collapse my-4" />,
  thead: (thProps: any) => <thead {...thProps} class="border-b border-border" />,
  tbody: (tbProps: any) => <tbody {...tbProps} />,
  tr: (trProps: any) => <tr {...trProps} class="border-b border-border/50 last:border-0" />,
  th: (thProps: any) => <th {...thProps} class="text-left font-semibold px-2 py-1.5 text-sm text-primary" />,
  td: (tdProps: any) => <td {...tdProps} class="px-2 py-1.5 text-sm text-secondary" />,
  ```
- **`markdown.tsx`** — no custom table overrides; falls through to default rehype HTML output
  with no styling.

Problems:
1. Wide tables overflow their container and break layout (no horizontal scroll).
2. Rows are hard to scan — no zebra striping or hover highlight.
3. GFM column alignment (`:---`, `:---:`, `---:`) is ignored.
4. No way to copy table data.
5. `markdown.tsx` tables are completely unstyled.

---

## Goals

1. **Visual polish** — consistent, readable table style across both rendering paths.
2. **Overflow handling** — wide tables scroll horizontally inside a contained wrapper.
3. **Column alignment** — respect GFM alignment markers from the parsed AST.
4. **Dedicated `TableBlock` component** — encapsulates all table logic, adds copy-as-CSV action.

---

## New File: `frontend/app/element/table-block.tsx`

A self-contained SolidJS component that renders the full `<table>` subtree.

### Props

```tsx
interface TableBlockProps {
    children: JSX.Element;   // thead + tbody passed through from markdown/streamdown
    class?: string;
}
```

### Behaviour

#### Overflow wrapper
Wrap the `<table>` in a `<div class="overflow-x-auto ...">` so wide tables scroll
horizontally. The wrapper should have a thin border and rounded corners matching the app
design language, acting as a visible container boundary.

```
┌─────────────────────────────────────────────────┐
│ Name        │ Status  │ Date         │ Score     │◄── horizontal scroll if needed
│─────────────┼─────────┼──────────────┼───────────│
│ foo         │ OK      │ 2026-05-01   │ 98        │
│ bar         │ FAIL    │ 2026-05-03   │ 12        │   ← zebra stripe on odd rows
└─────────────────────────────────────────────────┘
                                        [copy CSV]  ◄── top-right button
```

#### Copy-as-CSV
A small icon button (use existing `<IconButton>`) appears in the top-right corner of the
wrapper on hover. On click, it extracts all cell text via DOM traversal and writes a CSV
string to the clipboard via `clipboardWriteText` from `@/util/clipboard`.

CSV rules:
- Comma-separated; values containing `,` or `"` or newlines are quoted and escaped.
- First row = header row.
- No trailing newline.

The button should use the existing `<CopyButton>` pattern from `copybutton.tsx` — show a
checkmark for 1.5 s after a successful copy.

#### Zebra striping
Odd `<tbody>` rows get a subtle background tint: `bg-white/[0.02]` (works on dark themes;
adjust if light theme is added).

#### Row hover highlight
`<tbody> tr:hover` gets `bg-white/[0.04]`. Implemented via Tailwind group or direct class.

#### Header distinction
`<thead>` background: `bg-white/[0.04]` (slightly raised vs body). Bold font already in
place. Add `text-xs uppercase tracking-wide` to `<th>` for a stronger header feel.

---

## Column Alignment

### Streamdown path (`streamdown.tsx`)

Streamdown's component map receives the processed HTML props, which **do** include a `style`
attribute when the markdown source has alignment markers (markdown-it sets `text-align` via
inline style). Pass through the `style` prop on `<th>` and `<td>` without overriding it.

Current code strips alignment by spreading `{...thProps}` but then setting `class` which
does not interfere — however, if markdown-it emits `style="text-align: center"`, that will
already work. Verify this is preserved; if not, explicitly forward `tProps.style`.

### Markdown.tsx path (remark-gfm / rehype)

`remark-gfm` adds `align` attribute to `<th>`/`<td>` elements. `rehypeSanitize` strips
non-whitelisted attributes. Solution: add `"align"` (or `"style"`) to the allowed attributes
for `th` and `td` in the sanitize schema used in `markdown.tsx`.

Alternatively, add a custom rehype plugin that converts `align` attribute to inline
`style="text-align: ..."` before sanitization.

---

## Changes Required

### 1. New file: `frontend/app/element/table-block.tsx`

Full `TableBlock` component as described above.

### 2. `frontend/app/element/streamdown.tsx`

Replace the inline table component map entries (lines 259-264) with `TableBlock`:

```tsx
import { TableBlock } from "@/app/element/table-block";

// inside createMemo components:
table: (tProps: any) => <TableBlock {...tProps} />,
thead: (thProps: any) => <thead {...thProps} class="bg-white/[0.04]" />,
tbody: (tbProps: any) => <tbody {...tbProps} />,
tr: (trProps: any) => (
    <tr {...trProps} class="border-b border-border/40 last:border-0 hover:bg-white/[0.04] odd:bg-white/[0.02]" />
),
th: (thProps: any) => (
    <th
        {...thProps}
        class="text-left font-semibold px-3 py-2 text-xs uppercase tracking-wide text-primary"
    />
),
td: (tdProps: any) => <td {...tdProps} class="px-3 py-2 text-sm text-secondary" />,
```

### 3. `frontend/app/element/markdown.tsx`

Add custom component overrides for `table`, `thead`, `tbody`, `tr`, `th`, `td` inside
`markdownComponents` (around line 391), mirroring the streamdown treatment. Use `TableBlock`
for the outer wrapper.

Add custom component overrides for `table`, `thead`, `tbody`, `tr`, `th`, `td` inside
`markdownComponents` (around line 391). Wire in `TableBlock` for the outer `table` element.
The alignment-to-class rehype plugin (see Column Alignment section) handles `th`/`td`
alignment without touching the sanitize schema.

### 4. New rehype plugin: alignment-to-class (inline, ~20 lines)

```ts
// runs BEFORE rehypeSanitize in both paths
function rehypeAlignToClass() {
    return (tree: Root) => {
        visit(tree, "element", (node) => {
            if (node.tagName !== "th" && node.tagName !== "td") return;
            const align = node.properties?.align as string | undefined;
            if (!align) return;
            const cls = align === "center" ? "text-center"
                      : align === "right"  ? "text-right"
                      : "text-left";
            node.properties.className = [
                ...(Array.isArray(node.properties.className) ? node.properties.className : []),
                cls,
            ];
            delete node.properties.align;
        });
    };
}
```

Insert this before `rehypeSanitize` in `markdown.tsx`'s `rehypePlugins` array. For `streamdown.tsx`,
pass it as a custom `rehypePlugin` to `<Streamdown>` prepended before the default plugins.

### 5. `frontend/app/element/markdown.scss`

Add a `.table-block` rule for zebra striping and hover. Tailwind's `odd:` variant cannot be
applied from a parent component — SCSS is cleaner here (see Theming section).

---

## Non-Goals

- **Sortable columns** — out of scope for this iteration. Would require extracting cell data
  into a signal-driven store; defer to a follow-up spec.
- **Vertical scroll / max-height** — most tables in agent output are short; adding a fixed
  max-height would clip useful content. Revisit if user feedback calls for it.
- **Editable cells** — out of scope.
- **Pagination** — out of scope.

---

## Implementation Order

1. Create `table-block.tsx` with wrapper, copy-as-CSV, zebra/hover styling.
2. Wire into `streamdown.tsx` component map.
3. Wire into `markdown.tsx` component map + fix sanitize schema for alignment.
4. Manual smoke test: paste a GFM table with mixed alignment columns into an agent pane,
   verify horizontal scroll on a narrow pane, verify CSV copy output.

---

## Resolved: Open Questions

### Light theme — use `var(--hover-bg-color)` via `color-mix()`

**Finding**: All 8 themes (catppuccin, dracula, gruvbox, high-contrast, midnight, monokai,
nord, tokyo-night) are dark-only. No light theme exists. Every theme defines
`--hover-bg-color` as an accent-tinted semi-transparent color (e.g. `rgba(203,166,247,0.07)`
for catppuccin, `rgba(100,160,255,0.08)` for tokyo-night). The default theme uses
`rgba(255,255,255,0.1)`. Tailwind maps this to `--color-hover` in `@theme`.

**Solution**: Use CSS variables instead of `bg-white/[...]` hardcodes:

```scss
// In markdown.scss or a new table-block.scss
.table-block {
    tbody tr:nth-child(odd) {
        background: color-mix(in srgb, var(--hover-bg-color) 40%, transparent);
    }
    tbody tr:hover {
        background: var(--hover-bg-color);
    }
}
```

`color-mix()` is supported in Chrome 111+ — CEF bundles Chromium 120+ so this is safe.
If a light theme ships, it only needs to define `--hover-bg-color` with a dark-tinted value
and both zebra striping and hover adapt automatically. No `dark:` prefix required.

---

### Column alignment — `align` attribute, stripped by sanitize; fix via pre-sanitize plugin

**Finding** (confirmed via runtime test against the actual remark-gfm version in node_modules):

```bash
remark-gfm → remark-rehype → toHtml produces:
<th align="left">Left</th>
<th align="center">Center</th>
<th align="right">Right</th>
```

remark-gfm outputs the **deprecated HTML `align` attribute**, not `style="text-align: ..."`.

**Both paths strip it**:
- `markdown.tsx`: `rehypeSanitize({ ...defaultSchema })` — `align` is not in `defaultSchema`'s
  allowed attributes for `th`/`td`, so it is silently dropped.
- `streamdown.tsx`: Streamdown runs its own internal `rehype-sanitize` with `defaultSchema`
  before props reach our custom `th`/`td` component functions — same result.

**Solution**: A small rehype plugin (`rehypeAlignToClass`, ~20 lines) that runs **before**
`rehypeSanitize` in both paths. It visits `th`/`td` HAST nodes, reads `node.properties.align`,
maps it to a Tailwind class (`text-left` / `text-center` / `text-right`), appends it to
`node.properties.className`, and deletes `align`. Since `className` IS in `defaultSchema`'s
allowed attributes, it survives sanitization.

This avoids whitelisting `style` on table cells (which would open a CSS injection vector)
and avoids whitelisting the deprecated `align` attribute. See Changes Required §4 above.
