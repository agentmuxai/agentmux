# SPEC: User input visibility + startup-injection collapse

**Date:** 2026-05-24
**Author:** AgentA
**Status:** Draft

---

## TL;DR

Two coupled changes to user-message rendering in the agent pane:

1. **User input gets a high-contrast color.** The current `.agent-user-message` styling uses `var(--accent-color)` at 5% opacity for background and a 2px border — barely distinguishable from the surrounding text. Regular user-typed messages should pop. New CSS variable `--user-input-color` (with a strong, theme-aware default), used for a more saturated background tint and a bolder border.
2. **Startup injection collapses to one line by default.** The frontend's `onReadyFn` sends a long Markdown "session context" payload as the opening turn (per `SPEC_AGENT_STARTUP_SEQUENCE_2026_04_16.md`). It currently renders as a giant wall of text indistinguishable from a real user message. Render it like a tool block: single-line summary by default, hover-expand, click-to-pin.

Additionally: **regular user input must NEVER word-wrap.** The current `.agent-user-message-content pre` uses `white-space: pre-wrap` — long lines fold. The user's request is explicit: regular input is one-line-per-newline; long lines overflow horizontally (or get a scrollbar), they do not re-flow.

---

## Why now

Today on the latest portable, two visibility complaints surface in the same pane:

- Typed messages blend into the agent's reply stream. A user scrolling back through a long session can't quickly find their own inputs without `Ctrl+F`. The 5% accent tint is invisible on most themes.
- The startup injection (Markdown like `# Session Context`, `## Identity`, lists of accounts and peer agents — ~3k–8k characters) takes 20+ lines of pane real estate every time the user starts a fresh agent. It's noise after the first second.

Both have the same root: **user-input messages are a single rendering class today**, even though they fall into two distinct semantic buckets — the structured startup payload (a one-shot system prompt the user didn't really "type") and the user's actual typed turns.

---

## Current state of the code

### 1. Render site

`frontend/app/view/agent/virtualization/DocumentRow.tsx:249-261`

```tsx
<Show when={props.node() && props.node().type === "user_message"}>
    <div
        class="agent-user-message"
        classList={{
            "agent-user-message--collapsed":
                props.documentState().collapsedNodes.has(props.node().id),
        }}
    >
        <div class="agent-user-message-content">
            <pre>{(props.node() as Extract<DocumentNode, { type: "user_message" }>).message}</pre>
        </div>
    </div>
</Show>
```

The collapse state today is driven by a separate `collapsedNodes` set on the document state — toggled by an external hover-strip button, not by hover-on-the-message-itself. There's no `pinned` concept and no on-element hover signal. The startup payload is auto-collapsed once at document load via this same mechanism, but a click anywhere else expands it permanently.

### 2. SCSS

`frontend/app/view/agent/styles/_document-nodes.scss:598-624`

```scss
.agent-user-message {
    padding: 3px var(--space-1);
    background: color-mix(in srgb, var(--accent-color) 5%, transparent);
    border-left: 2px solid var(--accent-color);
    border-radius: 2px;

    .agent-user-message-content {
        pre {
            margin: 0;
            white-space: pre-wrap;       // current — wraps long lines
            overflow-wrap: anywhere;      // current — breaks long words
        }
    }

    &--collapsed .agent-user-message-content pre {
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    }
}
```

### 3. Startup payload

`frontend/app/view/agent/agent-view.tsx:497-539` (`onReadyFn`) builds a Markdown payload via `buildStartupPayload()` (`frontend/app/view/agent/startup/buildStartupPayload.ts`) and sends it via `handleSendMessage(payload)`. The payload always begins with the literal heading **`# Session Context`** as line 1 (see `buildStartupPayload.ts:46-119`). On the wire it's an ordinary `user_message` event; the stream-parser stores it as `type: "user_message"` with no distinguishing metadata.

### 4. Tool-block hover-expand pattern (reference)

`frontend/app/view/agent/components/ToolBlock.tsx:42-156` + `_document-nodes.scss:96-159`:

- `hovering` signal, `HOVER_ENTER_DELAY_MS = 150`.
- `pinned` prop (parent-managed) and `onTogglePin` callback.
- `autoExpanded()` for transient states (running, post-completion hold, pending approval).
- Combined: `expanded() = pinned || autoExpanded() || hovering()`.
- CSS classes `collapsed` / `expanded` / `pinned` on the outer div.
- Collapsed row uses `white-space: nowrap; overflow: hidden; text-overflow: ellipsis;` on `.agent-tool-summary`.
- Pinned state adds `border-left: 2px solid var(--accent-color)`.

This pattern is the model.

---

## Target state

### A. High-contrast user-input color

Add to the theme:

```scss
:root {
    /* Strong, saturated tint reserved for user-typed input. Distinct
     * from --accent-color so that future re-themings of the accent
     * (button highlights, link color) don't drag the user-input
     * surface with them. */
    --user-input-color: #00b4d8;          /* cyan-blue; high contrast on dark + light themes */
    --user-input-bg: color-mix(in srgb, var(--user-input-color) 14%, transparent);
    --user-input-border: var(--user-input-color);
}
```

Update `.agent-user-message`:

```scss
.agent-user-message {
    background: var(--user-input-bg);
    border-left: 3px solid var(--user-input-border);   /* bumped from 2 → 3 */
    /* …unchanged: padding, border-radius, user-select */
}
```

**Why a dedicated variable.** Re-using `--accent-color` couples typed-input visibility to the link/button accent. They are unrelated UX concerns; the user has explicitly asked for typed input to stand out, while the accent stays subtle on its own surfaces.

**Theme cohabitation.** `#00b4d8` is a perceptually distinct cyan that contrasts both against `--background-primary` (typical dark themes, #1c1c1c-ish) and against light surfaces (>4.5:1 luminance ratio on both). Themes that already define their own accent palette can override `--user-input-color` in their theme block.

### B. Regular user input — NEVER wraps

```scss
.agent-user-message-content {
    pre {
        margin: 0;
        white-space: pre;                /* CHANGED — no wrap, honor newlines as-is */
        overflow-x: auto;                /* horizontal scroll for over-wide single lines */
        overflow-y: hidden;
    }
}
```

Each `\n` in the message renders as its own line; the user controls wrapping by inserting newlines. Lines wider than the pane scroll horizontally. The standard browser-native horizontal scroll bar appears only on overflow.

`overflow-wrap: anywhere` is REMOVED — it was the rule that broke long words like URLs across lines. No word-wrap means the URL is one line and scrolls.

### C. Startup injection collapses on hover-expand

#### C.1 Distinguish startup from typed input

The stream-parser detects a startup message by content heuristic — the payload built by `buildStartupPayload()` always begins with `# Session Context` as the first line. We add a node-level metadata flag:

```ts
// frontend/app/view/agent/types.ts — UserMessageNode interface
interface UserMessageNode {
    type: "user_message";
    id: string;
    message: string;
    timestamp: number;
    /** True when the message is the auto-generated startup payload
     * (matches `^# Session Context\b`). Used by DocumentRow to
     * render with collapse-on-hover behavior. Set by the
     * stream-parser; never user-mutable. */
    isStartup?: boolean;
}
```

In `stream-parser.ts` `userMessageToNode()`:

```ts
const isStartup = /^# Session Context\b/.test(event.message);
return {
    type: "user_message",
    id: event.id,
    message: event.message,
    timestamp: event.timestamp,
    isStartup,
};
```

**Why content heuristic and not a wire-level flag.** `buildStartupPayload` is a frontend-only construct; the backend has no opinion on which user messages are "startup." A wire-level flag would require either (a) round-tripping the flag through the agent CLI's NDJSON output unchanged, which it doesn't do, or (b) duplicating local state in the frontend. The heuristic is one regex, the literal heading is owned by us in `buildStartupPayload.ts`, and a unit test pins the contract: any future renaming of the heading needs both files updated atomically.

#### C.2 Render with the tool-block hover pattern

Promote the user-message render to a small component (`UserMessageBlock.tsx`) that mirrors `ToolBlock`'s shape:

```tsx
// frontend/app/view/agent/components/UserMessageBlock.tsx

export const UserMessageBlock = (props: {
    node: UserMessageNode;
    pinned: boolean;
    onTogglePin: () => void;
}): JSX.Element => {
    const [hovering, setHovering] = createSignal(false);
    let enterTimer: ReturnType<typeof setTimeout> | undefined;

    const handleMouseEnter = () => {
        clearTimeout(enterTimer);
        enterTimer = setTimeout(() => setHovering(true), 150);
    };
    const handleMouseLeave = () => {
        clearTimeout(enterTimer);
        setHovering(false);
    };
    onCleanup(() => clearTimeout(enterTimer));

    // Startup gets collapse-by-default; regular user input is always
    // visible and unaffected by hover/pin.
    const collapsible = () => props.node.isStartup === true;
    const expanded = () => !collapsible() || props.pinned || hovering();

    return (
        <div
            class={clsx("agent-user-message", {
                "agent-user-message--collapsed": collapsible() && !expanded(),
                "agent-user-message--expanded": collapsible() && expanded(),
                "agent-user-message--startup": collapsible(),
                "agent-user-message--pinned": props.pinned,
            })}
            onMouseEnter={collapsible() ? handleMouseEnter : undefined}
            onMouseLeave={collapsible() ? handleMouseLeave : undefined}
            onClick={collapsible() ? props.onTogglePin : undefined}
        >
            <Show when={collapsible() && !expanded()}>
                {/* one-line collapsed summary */}
                <div class="agent-user-message-summary">
                    <span class="agent-user-message-icon">⓵</span>
                    <span class="agent-user-message-label">Session context</span>
                    <span class="agent-user-message-hint">(hover to peek · click to pin)</span>
                </div>
            </Show>
            <Show when={!collapsible() || expanded()}>
                <div class="agent-user-message-content">
                    <pre>{props.node.message}</pre>
                </div>
            </Show>
        </div>
    );
};
```

`DocumentRow` becomes a thin call site:

```tsx
<Show when={props.node().type === "user_message"}>
    <UserMessageBlock
        node={props.node() as UserMessageNode}
        pinned={props.documentState().pinnedNodes.has(props.node().id)}
        onTogglePin={() => props.dispatch({ type: "ToggleNodePin", id: props.node().id })}
    />
</Show>
```

#### C.3 SCSS additions

```scss
.agent-user-message {
    // …high-contrast base from §A…

    // Startup variant — single-line collapsed by default.
    &--startup {
        cursor: pointer;

        .agent-user-message-summary {
            display: flex;
            align-items: center;
            gap: var(--space-1);
            padding: var(--space-0-5) var(--space-1);
            white-space: nowrap;
            overflow: hidden;
            min-width: 0;
            user-select: none;

            .agent-user-message-icon {
                color: var(--user-input-color);
            }
            .agent-user-message-label {
                font-weight: 500;
                color: var(--user-input-color);
            }
            .agent-user-message-hint {
                color: var(--secondary-text-color);
                font-size: 0.85em;
                opacity: 0.75;
            }
        }
    }

    &--pinned {
        // Left border slightly thicker when pinned, mirroring ToolBlock.
        border-left-width: 4px;
    }

    // Hover-while-collapsed: temporarily shows expanded content but
    // keeps the visual hint that this row is collapsible.
    &--expanded.agent-user-message--startup {
        background: color-mix(in srgb, var(--user-input-color) 18%, transparent);
    }
}
```

The `--collapsed` rule from the current SCSS (`white-space: nowrap; overflow: hidden; text-overflow: ellipsis;`) is REPLACED by the explicit `--startup .agent-user-message-summary` block — there's no `pre` content to ellipsis-truncate; the summary is its own first-class element.

### D. Migration of the existing collapsedNodes set

`AgentDocumentView` currently populates `collapsedNodes` for any user message above a size threshold. After this spec lands, the startup payload is the only one that should auto-collapse, and it does so via `isStartup`, not via the size threshold. Two follow-up cleanups:

- `documentState.collapsedNodes` for user messages becomes vestigial. Remove the auto-collapse-on-size logic; let regular user input always render full.
- Add `documentState.pinnedNodes` (parallel structure) and a `ToggleNodePin` action.

Both ride with the implementation PR — no separate migration step.

---

## Behavior matrix

| State | Regular user input | Startup injection |
|---|---|---|
| Default render | Always expanded; pre-line text; horizontal scroll if line too wide | Collapsed: `⓵ Session context (hover to peek · click to pin)` one-line summary |
| Hover | No change | Expands to full content. 150ms enter-delay (matches ToolBlock). |
| Click | No-op (selectable text) | Toggles pin. Pinned → stays expanded after mouse leaves. |
| Word wrap | `white-space: pre` — newlines honored, no soft wrap | Same when expanded (no wrap inside the expanded body either) |
| Color | `--user-input-color` background tint at 14%, 3px left border | Same as regular when expanded; collapsed summary uses solid `--user-input-color` for icon + label |

---

## Edge cases

- **Startup message that crashes the heuristic.** A user who types `# Session Context` as their first message would get auto-collapsed. Acceptable: pinning is one click, and the user is unlikely to type that exact heading. A follow-up could promote the heuristic to a wire-level marker when/if the cost shows up.
- **Pre-existing sessions whose startup was already auto-collapsed via `collapsedNodes`.** After the migration, those rows re-render via the new path. They were collapsed before, they collapse now. No data loss; state-key on `node.id`.
- **Themes that already define `--accent-color`.** Untouched — only the user-message styling stops using `--accent-color`.
- **Very long single line of typed input** (e.g. a pasted URL). Renders one line, horizontal scroll inside the message's pre. The block doesn't grow vertically. Standard browser textarea behavior translates here.
- **Selection across multiple user messages.** `user-select: text` stays on the regular variant. The startup variant gets `user-select: none` on the summary row (selection inside the collapsed summary would be confusing and not useful), `text` on the expanded body.

---

## Tests

### L1 — Stream parser

- `userMessageToNode` sets `isStartup: true` when message starts with `# Session Context`.
- `userMessageToNode` sets `isStartup: false` for any other content (typed user message, agent reply quoted in user content, etc.).
- Test fixture pins the literal heading so a rename of the heading in `buildStartupPayload.ts` fails this test.

### L2 — UserMessageBlock component

- Regular user input (`isStartup` undefined / false): rendered expanded; no hover/click handlers attached; `pre` content visible.
- Startup input, no pin, no hover: only the summary row renders; `pre` content absent from the DOM.
- Startup input, hover-enter after 150ms: full `pre` content renders, summary hidden.
- Startup input, click on collapsed: `onTogglePin` called.
- Startup input, pinned: full content visible even after `mouseLeave`.

### L3 — Visual / manual

- Open a fresh agent. The first row is the collapsed `⓵ Session context` summary.
- Type "hello". The user message renders below, in the high-contrast user-input color.
- Hover the startup summary. Full session context appears under the cursor.
- Click the startup summary. It stays expanded; clicking again collapses.
- Type a very long single line (paste a 200-char URL). The line does not wrap; the message gets a horizontal scrollbar.
- Switch themes (light / dark / system). User-input color stays high-contrast on both.

---

## Order of delivery

One PR, three commits on `agenta/user-input-visibility`:

1. **CSS-only commit.** Add `--user-input-color` variable, swap `.agent-user-message` to use it, change `pre` to `white-space: pre`, remove `overflow-wrap`. Single SCSS file. Visual regression test or screenshot review.
2. **Stream-parser + types commit.** Add `isStartup` to `UserMessageNode`, set in `userMessageToNode`, with L1 tests.
3. **UserMessageBlock component commit.** Extract from `DocumentRow.tsx`, add hover/pin signals, wire pinnedNodes through `documentState`, retire the `collapsedNodes` user-message branch, with L2 tests.

Each commit is independently revertible; the SCSS commit alone delivers the high-contrast win without depending on the rest.

---

## Out of scope

- Agent-reply rendering (no change).
- Tool-block styling (separate spec — `tool-collapse.md`).
- Light/dark theme palette overhaul (a separate theming pass; this spec only adds one variable to the existing theme block).
- Markdown rendering of the startup payload (currently raw `<pre>`; making it a rendered `Markdown` block is a follow-up that doesn't change the collapse behavior).
- Selectable summary row content (the collapsed summary is a fixed `Session context` label; we don't try to extract a preview from the body).

---

## Related

- `docs/specs/tool-collapse.md` — the source pattern for hover-expand-pin, with the same 150ms enter-delay and the same CSS-class trio (`collapsed` / `expanded` / `pinned`).
- `docs/specs/SPEC_AGENT_STARTUP_SEQUENCE_2026_04_16.md` — the contract that defines what the startup payload contains and when it's sent. The collapse target is exactly the message that spec produces.
- `feedback_agent_pane_tool_display` (user memory) — "Tool blocks ONE LINE by default, expand on hover (instant), click to pin." The same rule, now applied to the startup injection.
