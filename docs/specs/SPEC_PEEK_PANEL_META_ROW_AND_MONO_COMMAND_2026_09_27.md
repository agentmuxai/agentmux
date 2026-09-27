# SPEC: hover peek panel — time and tokens on one right-aligned line, command in the tool call's colour and monospace font

**Author:** lark
**Date:** 2026-09-27
**Status:** active. §1–§6 implemented (this PR); §7 is a separate follow-up, not started.
**Related:** `SPEC_TRANSCRIPT_NODE_HOVER_PEEK_ALL_KINDS_2026_08_25.md` (the peek on every node kind),
`SPEC_PEEK_OVERLAY_MOUSE_Y_TRACKING_2026_09_03.md` (the panel pinned right that follows the mouse's vertical
position), `SPEC_AGENT_PANE_HOVER_CLOSE_FOCUS_REFINEMENTS_2026_09_23.md` §1 (the command line in the peek).

All line numbers are against `main` at `af2080e9e`.

---

## 1. Request (user, 2026-09-27)

Refine the hover panel that appears pinned to the right of a transcript row and follows the mouse vertically:

1. The time and the token count sit **on the same line**, **pinned to the right**.
2. The full, word-wrapped command is in the **same colour and the same monospace font as in the tool call**:
   green for Bash, yellow for Write/Edit, and so on. Today it renders in the default text colour and a
   proportional font. (An earlier revision of this request said "green" everywhere; corrected by the user the
   same day to "the same colour as the tool call".)
3. Line breaks in a path may fall **anywhere**; don't restrict where it breaks.
4. Nodes without a command also get time and tokens on one line.

---

## 2. Today

`PeekOverlay.tsx` renders the panel into `document.body` (a Portal) and positions it; each node kind supplies
the contents. Every kind stacks its lines as separate blocks:

```tsx
<Show when={peekTimeText()}>     <div class="agent-node-peek-tooltip-meta">{peekTimeText()}</div></Show>
<Show when={peekEstimateText()}> <div class="agent-node-peek-tooltip-meta">{peekEstimateText()}</div></Show>
<Show when={cmdText()}>          <div class="agent-node-peek-tooltip-body">{cmdText()}</div></Show>
```

| Node kind | File | Lines today |
|---|---|---|
| Tool call | `ToolBlock.tsx` ~459–469 | time, tokens, command (`header().detail`: command, path or query) |
| Persistent shell | `PersistentShellBlock.tsx` ~204–212 | time, tokens, shell command |
| Agent message | `AgentMessageBlock.tsx` ~78–84 | time, tokens |
| Jekt | `JektBubble.tsx` ~131–137 | time, tokens |
| Markdown / thinking | `MarkdownBlock.tsx` ~179–188 | time, tokens |
| User message | `UserMessageBlock.tsx` ~268–277 | time, tokens (its second overlay, the stretched body preview, is separate and out of scope) |
| Ambient narration | `AmbientNarrationBlock.tsx` ~61–67 | time, then a note ("AgentMux ambient narration (…) — not written by the model") |

Time is `"<exact time> · <n>m ago"` (`formatExactTime` / `formatTimeAgo`), and tokens are `"~<n> tok (est.)"`.

Styles (`styles/_document-nodes.scss` ~2405–2455):

```scss
.agent-node-peek-tooltip-meta { font-size: 13px; line-height: 1.4; color: var(--secondary-text-color); white-space: nowrap; }
.agent-node-peek-tooltip-body { white-space: pre-wrap; overflow-wrap: break-word;
                                font-family: var(--fixed-font, monospace);   // ← invalid, see §3
                                font-size: 13px; line-height: 1.6; color: var(--main-text-color); }
.agent-node-peek-overlay      { display: flex; flex-direction: column; gap: 4px; width: max-content; … }
```

`PeekOverlay` caps the panel at the row's width (inline `max-width`, ~line 287) and right-aligns it to the row.

---

## 3. Why the command is not monospace (root cause)

Two facts combine:

1. **The Portal escapes the pane's font.** Tool rows are monospace because `.agent-view` sets
   `font-family: var(--font-mono)` (`agent-view.scss` ~419) and everything in the pane inherits it. The peek panel
   is rendered at `document.body`, outside `.agent-view`, so it inherits the app's **sans** font instead.
2. **The body's own `font-family` is invalid.** `--fixed-font` is a `font` *shorthand* value
   (`normal var(--font-mono-size) / normal var(--font-mono)`, `theme.scss` ~70), not a family. Passed to
   `font-family:` the declaration is invalid at computed-value time, the browser drops it (the `monospace`
   fallback inside `var()` does not apply, because the variable *is* defined), and the element keeps the
   inherited sans. `theme.scss` ~60–68 warns about exactly this: *"passing these to `font-family:` is invalid
   CSS … use `var(--font-mono)`."*

The fix is `font-family: var(--font-mono)`, the canonical family token and the one `.agent-view` uses.

---

## 4. Change

### 4.1 One meta line: time and tokens, right-aligned

Replace the two meta `<div>`s with **one** row, rendered by a small shared component so all seven call sites
stay identical:

```tsx
// components/PeekMetaRow.tsx
export const PeekMetaRow = (props: { time?: string | null; tokens?: string | null }) => (
    <Show when={props.time || props.tokens}>
        <div class="agent-node-peek-tooltip-meta">
            <Show when={props.time}><span class="agent-node-peek-tooltip-time">{props.time}</span></Show>
            <Show when={props.tokens}><span class="agent-node-peek-tooltip-tokens">{props.tokens}</span></Show>
        </div>
    </Show>
);
```

```scss
.agent-node-peek-tooltip-meta {
    display: flex;
    justify-content: flex-end;   // pinned to the right edge of the panel
    gap: var(--space-2);         // between time and tokens
    font-size: 13px;
    line-height: 1.4;
    color: var(--secondary-text-color);
    white-space: nowrap;
}
```

- Order is time, then tokens: `9:14:02 PM · 3m ago    ~1.2k tok (est.)`.
- When only one of the two exists, it still sits at the right.
- It stays the first line of the panel, above the command, as today.
- With a command, the panel is as wide as the command wraps to (up to the row's width), and the meta line is
  pinned to its right edge. Without a command, the panel shrink-wraps to the meta line, which already sits at
  the row's right edge.
- The ambient-narration note is not time or tokens: it stays its own line below the meta row, left-aligned, as
  a new `agent-node-peek-tooltip-note` class carrying today's meta styling.

### 4.2 The command: the tool call's colour, monospace, breaks anywhere

```scss
.agent-node-peek-tooltip-body {
    white-space: pre-wrap;
    word-break: break-all;           // was overflow-wrap: break-word
    font-family: var(--font-mono);   // was the invalid var(--fixed-font, monospace)
    font-size: 13px;
    line-height: 1.6;
    color: var(--main-text-color);   // default; per-tool colour below
}
```

**Colour: the same as the tool call's row.** The row colours its command through
`.agent-tool-block[data-tool=X] .agent-tool-name` (`_document-nodes.scss` ~656–665; the detail span sits inside
`.agent-tool-name` and inherits it):

| `data-tool` | Colour |
|---|---|
| `bash` | `--term-bright-green` |
| `read` | `--accent-color` |
| `write`, `edit` | `--warning-color` (yellow) |
| `grep`, `glob` | `--term-bright-cyan` |
| `agent`, `workflow` | `--term-bright-magenta` |
| `task`, `other` | `--secondary-text-color` |
| anything else (e.g. `webfetch`, `websearch`) | no rule: the default text colour |

The popover is Portal-rendered outside the row, so it can't inherit that colour. Two changes carry it across:

1. `ToolBlock.tsx` puts the same attribute on the popover body that it puts on the row:
   `<div class="agent-node-peek-tooltip-body" data-tool={props.node.tool.toLowerCase()}>`.
2. **One colour table for both.** Move the ten row rules into an SCSS map,
   `$tool-name-colors: (bash: var(--term-bright-green), read: var(--accent-color), …)`, and generate both the
   existing row rules and the new `.agent-node-peek-tooltip-body[data-tool="…"]` rules from it with `@each`. A
   colour change then moves the row and the popover together, and they can't drift. The compiled CSS for the
   row rules must be byte-for-byte what it is today (checked by compiling before and after, §6), so nothing
   about the row changes.

The persistent-shell popover shows the shell's command, which its row renders as `.agent-shell-header-cmd` in
`--secondary-text-color`. Its popover body gets a modifier class, `agent-node-peek-tooltip-body--shell`, with that
colour.

- **Font:** `var(--font-mono)`, the same family as the tool row. The size stays 13px: the peek's size was set
  deliberately to read like a native tooltip (`SPEC_TRANSCRIPT_NODE_HOVER_PEEK_2026_08_03.md` §2.2a), and the
  panel is already scaled with the pane's zoom by `PeekOverlay`.
- **Breaks anywhere:** `overflow-wrap: break-word` only breaks inside a word when the whole word can't fit on a
  line of its own, so a long path jumps to a new line first and leaves a ragged gap. `word-break: break-all`
  breaks at whatever character reaches the edge, so every line fills. `pre-wrap` keeps real newlines and spaces.

### 4.3 Call sites

Each of the seven files in §2 swaps its two meta `<Show>` blocks for
`<PeekMetaRow time={peekTimeText()} tokens={peekEstimateText()} />`. Nothing else in them changes: which texts
exist, the `hasAnyPeekContent` gate, and positioning are untouched.

---

## 5. Scope

**The substance of this spec is the hover popover panel.** Nothing visible outside it changes: the transcript
rows, the tool-call header and its colours, and the expanded tool panels look exactly as they do today. The
component edits only replace the markup inside each `<PeekOverlay>` (plus the `data-tool` attribute on the
popover body). The one non-popover source edit, moving the row colours into the shared `$tool-name-colors` map
(§4.2), must compile to identical CSS for the row.

- **In:** the popover panel's content and styling, for every node kind listed in §2.
- **Out:**
  - Positioning, mouse-Y tracking, enter delay, zoom handling (`PeekOverlay.tsx`).
  - The user message's stretched body preview (`.agent-user-message-peek-overlay`).
  - The tool row's appearance.
  - The same font bug elsewhere: §7, a separate follow-up.

---

## 6. Tests

- Update the existing peek tests that count meta lines (`AgentMessageBlock`, `JektBubble`, `MarkdownBlock`,
  `PersistentShellBlock`, `ToolBlock`, and `UserMessageBlock` if it counts). They now expect **one**
  `.agent-node-peek-tooltip-meta` holding `.agent-node-peek-tooltip-time` and `.agent-node-peek-tooltip-tokens`,
  and when there's no timestamp, one row with only the tokens span.
- `PeekMetaRow.test.tsx`: both, time only, tokens only, neither (renders nothing).
- `AmbientNarrationBlock`: the note is a separate `.agent-node-peek-tooltip-note` line.
- `ToolBlock.test.tsx`: the popover body carries the row's `data-tool` value (e.g. `bash`, `write`).
- `PersistentShellBlock.test.tsx`: the popover body has `agent-node-peek-tooltip-body--shell`.
- Stylesheet contract, in the style of `styles/tool-panel-height.test.ts`: the body rule uses
  `font-family: var(--font-mono)` and `word-break: break-all`, and doesn't use `--fixed-font`; the meta rule is
  `display: flex` with `justify-content: flex-end`.
- Colour parity, on the **compiled** CSS (sass): for every `data-tool` value, the
  `.agent-node-peek-tooltip-body[data-tool=X]` colour equals the `.agent-tool-block[data-tool=X] .agent-tool-name`
  colour; and the compiled row rules are identical to `main`'s.
- Live check in a dev build: hover a Bash call with a long command (green), a Write or Edit call (yellow), a
  Read call with a long path (blue), a WebFetch call (default text colour), a persistent shell, and an agent
  message. Check that time and tokens sit on one line at the right, the command is monospace in the row's
  colour, and a long path fills each line before breaking.

---

## 7. Follow-up (separate change): the same font bug elsewhere

The popover body is one of **47** declarations in 20 stylesheets that pass a `font` shorthand token
(`--fixed-font` or `--base-font`) to `font-family:`, which is invalid (§3). Examples: `.agent-tool-panel`,
`_shell-node.scss`, `_decision-panel.scss`, `ErrorBanner.scss`, `command-palette.scss`, the status bar's
`_token-usage.scss` and `_cpu-cores-popover.scss`, and the memory editor.

- Inside `.agent-view` the bug is mostly hidden: the element inherits the pane's mono font anyway, so it looks
  right by accident.
- Outside it (Portals, modals, the status bar, other panes) the element silently gets the app's sans font
  wherever monospace was intended.

Fix as its own PR after this one: replace `var(--fixed-font…)` with `var(--font-mono)` and `var(--base-font…)`
with `var(--font-sans)` in each `font-family:` declaration, and add a stylelint rule in `.stylelintrc.json`
rejecting those two tokens in `font-family:` so it can't come back. Changes are visible only where an element was
rendering in the wrong font; each should be checked in a dev build.
