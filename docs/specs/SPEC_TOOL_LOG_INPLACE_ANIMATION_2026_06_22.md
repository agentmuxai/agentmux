# SPEC_TOOL_LOG_INPLACE_ANIMATION_2026_06_22

> **Superseded (generalized) by
> [`SPEC_TOOL_LOG_UNIVERSAL_ANIMATION_COLLAPSE_2026_07_27.md`](SPEC_TOOL_LOG_UNIVERSAL_ANIMATION_COLLAPSE_2026_07_27.md).**
> This spec's fix only collapsed a chunk whose entire content, after trim,
> was a single bare spinner glyph — a spinner glyph or progress text
> trailing/leading other text on the same line (`Installing... ⠋`,
> `Downloading (45%)`) was not covered. The follow-up spec closes that gap
> at both the backend (`bash_wrap.rs`) and frontend (`output-cap.ts`)
> layers. Kept here as the historical record of the initial, narrower fix.

## Problem

When agents run CLI tools, many emit spinner animations as a sequence of
single-character frames on individual lines — one animation character per chunk:

```
⠋
⠙
⠹
⠸
⠼
```

Today each frame becomes its own `<pre class="agent-tool-log-line">` element,
producing a vertical stack of spinner characters in the expanded tool overlay.
The expected behaviour (matching a real terminal) is that each new frame
**replaces** the previous one in place, with the final character freezing
statically when the tool finishes — exactly like a terminal spinner.

Prior work: PR #1351 added Rust-side `pending_cr_line` collapsing for
`\r`-prefixed frames. The remaining case is tools that output frames as plain
`char\n` lines with no carriage return — each frame arrives as a separate chunk
with no `\r`, so the Rust pass doesn't touch them. This spec handles that case
at the frontend render layer.

---

## Approach: Frontend-only spinner detection

No backend or protocol changes. In `ChunkList` (inside `ToolOverlayLog.tsx`),
a `createMemo` walks the capped chunk array and collapses consecutive runs of
known spinner characters:

- A **trailing run** (at the end of the chunk list while the tool is still
  streaming) is rendered as a single `<pre>` whose text Solid updates in place
  as each new frame arrives. When the tool finishes, that `<pre>` freezes on
  the last character — no CSS animation, just the static final frame.

- A **completed run** (a spinner run followed by non-spinner output) collapses
  to its last frame and is rendered as a normal frozen `<pre>`.

No backend changes, no protocol changes, no new CSS animations.

---

## Detection: `SPINNER_CHARS`

```typescript
const SPINNER_CHARS = new Set([
    // Braille (ora, listr, tqdm, …)
    '⠋','⠙','⠹','⠸','⠼','⠴','⠦','⠧','⠇','⠏',
    '⣾','⣽','⣻','⢿','⡿','⣟','⣯','⣷',
    // Quarter-circle
    '◐','◓','◑','◒','◴','◷','◶','◵',
    // ASCII classic
    '-','\\','|','/',
]);

function isSpinnerChar(s: string): boolean {
    return SPINNER_CHARS.has(s.trim());
}
```

A chunk matches if its content (after `capChars` + `trim()`) is a single
character in the set. The `trim()` handles chunks that arrived with a trailing
newline stripped by the Rust layer.

---

## Implementation: `ChunkList` in `ToolOverlayLog.tsx`

```tsx
function ChunkList(props: ChunkListProps): JSX.Element {
    const cap = createChunkCapper();

    const view = createMemo(() => {
        const { chunks, hiddenLines } = cap(props.chunks);
        const display: LogChunk[] = [];
        let spinnerSlot: { content: string; kind: string } | null = null;

        for (let i = 0; i < chunks.length; i++) {
            const chunk = chunks[i];
            const trimmed = capChars(chunk.content).trim();
            if (isSpinnerChar(trimmed)) {
                // Consume the entire consecutive spinner run.
                let last = chunk;
                while (
                    i + 1 < chunks.length &&
                    isSpinnerChar(capChars(chunks[i + 1].content).trim())
                ) {
                    i++;
                    last = chunks[i];
                }
                const lastFrame = capChars(last.content).trim();
                if (i === chunks.length - 1) {
                    // Trailing run — live slot, updates in place.
                    spinnerSlot = { content: lastFrame, kind: last.kind };
                } else {
                    // Completed run — freeze last frame as a static line.
                    display.push({ ...last, content: lastFrame });
                    spinnerSlot = null;
                }
            } else {
                display.push(chunk);
                spinnerSlot = null;
            }
        }

        return { display, spinnerSlot, hiddenLines };
    });

    return (
        <>
            <Show when={view().hiddenLines > 0}>
                <OutputHiddenMarker hidden={view().hiddenLines} noun="line" from="tail" />
            </Show>
            <For each={view().display}>
                {(chunk) => (
                    <pre class={`agent-tool-log-line ${KIND_CLASS[chunk.kind] ?? ""}`}>
                        {capChars(chunk.content)}
                    </pre>
                )}
            </For>
            <Show when={view().spinnerSlot !== null}>
                <pre class={`agent-tool-log-line ${KIND_CLASS[view().spinnerSlot?.kind ?? ""] ?? ""}`}>
                    {view().spinnerSlot?.content}
                </pre>
            </Show>
        </>
    );
}
```

Solid's fine-grained reactivity updates only the text node inside the spinner
`<pre>` when `spinnerSlot.content` changes — the element itself stays mounted.
When the tool finishes and no more chunks arrive, the last frame simply remains.
If non-spinner output follows the spinner run, the slot's `<Show>` unmounts and
the last frame moves into `display` as a static frozen line.

---

## Same pattern in `PersistentShellBlock.tsx`

Apply the same `isSpinnerChar` + collapse logic to the `<For>` over
`visibleChunks()` in `PersistentShellBlock.tsx`. The spinner slot `<pre>` there
wraps `<LinkifiedText>` like the other lines.

---

## Behaviour summary

| State | Display |
|---|---|
| Spinner frames arriving (streaming) | Single `<pre>` updating in place |
| Non-spinner output after spinner run | Last spinner frame frozen + new line appended |
| Tool finishes mid-spinner | Last spinner frame frozen in place, no CSS spin |
| No spinner chars in output | Identical to today — no overhead |
| Multiple disjoint spinner runs | Each collapses independently |

---

## Files changed

- `frontend/app/view/agent/components/ToolOverlayLog.tsx` — update `ChunkList`
- `frontend/app/view/agent/components/PersistentShellBlock.tsx` — same collapse
- `docs/specs/SPEC_TOOL_LOG_INPLACE_ANIMATION_2026_06_22.md` — this file
