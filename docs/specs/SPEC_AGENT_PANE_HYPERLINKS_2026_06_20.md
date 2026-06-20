# SPEC: Aggressive Hyperlink Detection in Agent Pane

**Date:** 2026-06-20  
**Status:** Draft  
**Scope:** Frontend only. Two implementation paths; ~100 lines total across 3–4 files.

---

## 1. Goal

Any URL that appears in agent output — including those **without** an `http://` prefix
(bare domains like `github.com/foo`, local dev servers like `localhost:3000`,
IPs with ports like `127.0.0.1:8080`) — should be rendered as a clickable hyperlink
that opens in the system browser via the existing `openLink()` / `openExternal` IPC.

---

## 2. Scope: All Text-Bearing Node Types

| Node type | Rendered by | Text format | Linkify path |
|-----------|-------------|-------------|--------------|
| `MarkdownNode` (assistant output) | `MarkdownBlock` → `Markdown` | Markdown AST via rehype | rehype plugin (§4) |
| `MarkdownNode` with `metadata.thinking: true` | Same as above | Same | Same rehype plugin |
| `AgentMessageNode` | `AgentMessageBlock` | Plain `<pre>` text | `LinkifiedText` component (§5) |
| `UserMessageNode` | `UserMessageBlock` | Plain `<pre>` text | `LinkifiedText` component (§5) |
| `ShellNode` output lines | `PersistentShellBlock` (or equivalent) | Plain text per line | `LinkifiedText` component (§5) |

Reasoning (thinking) blocks are `MarkdownNode` with `metadata.thinking = true` — they
go through the exact same `Markdown` component as regular assistant content, so the
rehype plugin covers them automatically.

---

## 3. Library: `linkify-it`

**Choice:** `linkify-it` (not `linkifyjs`, not `autolinker`)

**Rationale:**
- 11.9M weekly npm downloads; actively maintained (v5.x)
- Used internally by `markdown-it` (the most-deployed markdown renderer)
- TypeScript: `@types/linkify-it` available on DefinitelyTyped
- ~3–5 KB gzipped — smallest in class
- Handles bare domains, `localhost:PORT`, `//`-relative URLs, Unicode TLDs
- Correctly strips trailing punctuation (`.`, `,`, `)`, `?`, `!`) without configuration

**Install:**
```
pnpm add linkify-it
pnpm add -D @types/linkify-it
```

**Configuration:**
```typescript
import LinkifyIt from "linkify-it";

const linkify = new LinkifyIt();
// Match bare domains (github.com, localhost:3000, etc.)
linkify.set({ fuzzyLink: true, fuzzyEmail: false });
```

`fuzzyLink: true` enables matching without `http://`. `fuzzyEmail: false` avoids
false positives on `name@host` patterns in shell output.

---

## 4. Path A — MarkdownNode (including thinking blocks)

### 4.1 Where to insert

`markdown.tsx` builds a unified processor. The rehype plugin array (lines 476–513)
currently runs in this order:

```
rehypeRaw
rehypeHighlight (conditional)
rehypeAlignToClass
rehypeSanitize(...)    ← our plugin goes HERE, just before this
rehypeSlug
```

Insert **before** `rehypeSanitize` so the sanitizer can see and preserve our
injected `<a href>` elements. (`defaultSchema` already allows `<a>` with `href`.)

Insert **after** `rehypeHighlight` so we don't attempt to linkify inside
already-tokenized code spans.

### 4.2 New file: `frontend/app/element/rehype-linkify.ts`

```typescript
import type { Root, Element, Text } from "hast";
import type { Plugin } from "unified";
import { visit, SKIP } from "unist-util-visit";
import LinkifyIt from "linkify-it";

const linkify = new LinkifyIt();
linkify.set({ fuzzyLink: true, fuzzyEmail: false });

// Ancestor tag names that mean "don't linkify inside here"
const SKIP_TAGS = new Set(["a", "code", "pre", "script", "style"]);

export const rehypeLinkify: Plugin<[], Root> = () => (tree) => {
    visit(tree, "text", (node: Text, index, parent) => {
        if (!parent || index == null) return;

        // Skip nodes whose nearest-ancestor is a skipped tag
        // (unist-util-visit provides the immediate parent; we need to walk up.
        // A pragmatic shortcut: visit() with ancestors tracking.)
        // Instead: check if any ancestor in the walk is a skip tag.
        // We do this via the parent reference provided at call time.
        const parentElement = parent as Element;
        if (parentElement.type === "element" && SKIP_TAGS.has(parentElement.tagName)) {
            return SKIP;
        }

        const matches = linkify.match(node.value);
        if (!matches) return;

        const newNodes: (Text | Element)[] = [];
        let lastIndex = 0;

        for (const match of matches) {
            if (match.index > lastIndex) {
                newNodes.push({ type: "text", value: node.value.slice(lastIndex, match.index) });
            }
            const href = match.url.startsWith("//")
                ? "https:" + match.url
                : match.url.includes("://")
                ? match.url
                : "https://" + match.url;

            newNodes.push({
                type: "element",
                tagName: "a",
                properties: { href },
                children: [{ type: "text", value: match.text }],
            } as Element);
            lastIndex = match.lastIndex;
        }

        if (lastIndex < node.value.length) {
            newNodes.push({ type: "text", value: node.value.slice(lastIndex) });
        }

        parent.children.splice(index, 1, ...newNodes);
        return SKIP; // don't re-visit the inserted nodes
    });
};
```

> **Ancestor check note:** `visit()` from `unist-util-visit` provides only the
> immediate parent. For deeper skip-tag detection (e.g. `<pre><code>text</code></pre>`),
> use the `ancestors` variant: `visit(tree, "text", visitor, ancestors)` and check
> `ancestors.some(a => a.type === "element" && SKIP_TAGS.has(a.tagName))`.
> The `unist-util-visit-parents` package provides this. Use it instead of
> `unist-util-visit` to get reliable code-block exclusion.

### 4.3 Wire into `markdown.tsx`

In the `rehypePlugins` array (around line 476), change:

```typescript
// Before:
rehypeAlignToClass,
() => rehypeSanitize({...}),

// After:
rehypeAlignToClass,
rehypeLinkify,
() => rehypeSanitize({...}),
```

Import at top:
```typescript
import { rehypeLinkify } from "./rehype-linkify";
```

### 4.4 Click handling — no change needed

The `markdownComponents` registry in `markdown.tsx` (line 406) maps every `<a>`
element to the existing `Link` component:

```typescript
a: (props: any) => <Link props={props} setFocusedHeading={setFocusedHeading} />,
```

`Link` already calls `openLink(href)` for all non-anchor hrefs. Our injected
`<a href="https://...">` elements will automatically route through this handler.
No additional click wiring needed.

---

## 5. Path B — Plain Text Nodes (`AgentMessageBlock`, `UserMessageBlock`, Shell)

These render raw text in `<pre>` tags. No markdown pipeline is involved.

### 5.1 New file: `frontend/app/element/linkified-text.tsx`

```typescript
import { createMemo, For } from "solid-js";
import LinkifyIt from "linkify-it";
import { openLink } from "@/app/store/global";

const linkify = new LinkifyIt();
linkify.set({ fuzzyLink: true, fuzzyEmail: false });

type Segment = { text: string; url?: string };

function linkifySegments(text: string): Segment[] {
    const matches = linkify.match(text);
    if (!matches) return [{ text }];

    const segments: Segment[] = [];
    let lastIndex = 0;

    for (const match of matches) {
        if (match.index > lastIndex) {
            segments.push({ text: text.slice(lastIndex, match.index) });
        }
        const href = match.url.startsWith("//")
            ? "https:" + match.url
            : match.url.includes("://")
            ? match.url
            : "https://" + match.url;
        segments.push({ text: match.text, url: href });
        lastIndex = match.lastIndex;
    }

    if (lastIndex < text.length) {
        segments.push({ text: text.slice(lastIndex) });
    }

    return segments;
}

export const LinkifiedText = (props: { text: string }) => {
    const segments = createMemo(() => linkifySegments(props.text));
    return (
        <For each={segments()}>
            {(seg) =>
                seg.url ? (
                    <a
                        href={seg.url}
                        onClick={(e) => {
                            e.preventDefault();
                            openLink(seg.url!);
                        }}
                    >
                        {seg.text}
                    </a>
                ) : (
                    <>{seg.text}</>
                )
            }
        </For>
    );
};
```

### 5.2 `AgentMessageBlock.tsx`

Replace the plain `<pre>` content:

```typescript
// Before:
<pre class="agent-message-body">{props.node.message}</pre>

// After:
<pre class="agent-message-body">
    <LinkifiedText text={props.node.message} />
</pre>
```

### 5.3 `UserMessageBlock.tsx`

Locate the user message `<pre>` element (around line 244) and wrap similarly:

```typescript
// Before:
<pre>{props.node.message}</pre>

// After:
<pre><LinkifiedText text={props.node.message} /></pre>
```

### 5.4 Shell output

If shell lines are rendered as raw text strings, wrap them with `<LinkifiedText>` in
the same way. Investigate `PersistentShellBlock` or the relevant shell renderer to
find the exact `<pre>` / text render site.

---

## 6. URL Normalization

All injected `href` values follow this rule (applied in both `rehype-linkify.ts` and
`linkified-text.tsx`):

```
match.url starts with "//"  →  "https:" + match.url
match.url includes "://"    →  use as-is
otherwise (bare domain)     →  "https://" + match.url
```

`localhost:3000` → `https://localhost:3000`  
`github.com/foo` → `https://github.com/foo`  
`http://example.com` → `http://example.com` (unchanged)

---

## 7. Security

- `openLink()` delegates to `getApi().openExternal()` → Chromium / OS default handler.
  The OS will refuse `javascript:` and `data:` URIs for browser launch.
- As a defense-in-depth layer, filter before calling `openLink`:
  ```typescript
  const SAFE_SCHEMES = /^(https?|ftp|mailto|ssh|file):\/\//i;
  const isSafe = (url: string) => SAFE_SCHEMES.test(url) || url.startsWith("//");
  ```
  Only call `openLink` when `isSafe(href)` is true.
- `rehype-sanitize` (already in the markdown pipeline) strips `javascript:` hrefs from
  any injected `<a>` elements that somehow slip through.
- `linkify-it` with `fuzzyLink` will not match bare words without TLDs or ports.
  False positive rate on typical LLM prose is very low.

---

## 8. Edge Cases

| Scenario | Behavior |
|----------|----------|
| Trailing `.` or `,` after URL | `linkify-it` strips correctly by default |
| URL in parentheses: `(https://example.com)` | `linkify-it` strips trailing `)` |
| Markdown explicit link `[text](url)` | Already an `<a>` in HAST before our plugin runs; `SKIP_TAGS` includes `a` → no double-linkification |
| URL in fenced code block | HAST `<pre><code>` ancestry → skip via ancestor check |
| URL in inline code `` `localhost:3000` `` | HAST `<code>` ancestry → skip |
| `localhost` with no port | Not matched (no TLD, no port — not a URL) |
| `localhost:3000` | Matched via `fuzzyLink` + port detection |
| `127.0.0.1:8080` | Matched |
| Bare IPv4 `192.168.1.1` (no port) | Not matched (avoids IP false positives) |
| Path like `./src/foo.ts` | Not matched (no TLD/port) |
| Email `user@example.com` | Not matched (`fuzzyEmail: false`) |
| Same URL in streaming update | `createMemo` in `LinkifiedText` recomputes; rehype plugin re-runs on changed text — idempotent |

---

## 9. Styling

Add to `_document-nodes.scss` (or the global link style):

```scss
// Linkified URLs in agent output
.agent-markdown-block a,
.agent-message-body a,
.agent-user-message a {
    color: var(--link-color, #60a5fa);
    text-decoration: underline;
    text-decoration-color: color-mix(in srgb, var(--link-color, #60a5fa) 40%, transparent);
    cursor: pointer;

    &:hover {
        text-decoration-color: var(--link-color, #60a5fa);
    }
}
```

The `.thinking-block a` inherits from `.agent-markdown-block a` because thinking
blocks are wrapped in `.agent-markdown-block.thinking-block`.

---

## 10. Implementation Steps

1. `pnpm add linkify-it && pnpm add -D @types/linkify-it` (in `frontend/`)
2. Create `frontend/app/element/rehype-linkify.ts` (§4.2) — use `unist-util-visit-parents` for ancestor checking
3. Wire `rehypeLinkify` into `markdown.tsx` rehype plugin array (§4.3)
4. Create `frontend/app/element/linkified-text.tsx` (§5.1)
5. Update `AgentMessageBlock.tsx` (§5.2)
6. Update `UserMessageBlock.tsx` (§5.3)
7. Update shell output renderer (§5.4)
8. Add link styles to `_document-nodes.scss` (§9)
9. Test: paste `check out github.com/foo and localhost:3000` in user message → both clickable
10. Test: URL inside code block → NOT linkified
11. Test: thinking block with URL → linkified
12. Test: existing `[text](url)` markdown links → unaffected
13. Changeset: `patch "feat(agent-pane): aggressive URL hyperlinking in all text node types"`

**Total diff estimate:** ~120 lines across 5 files + 1 new file + 1 new file.
