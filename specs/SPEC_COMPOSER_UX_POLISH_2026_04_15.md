# Spec: Composer UX Polish — controls above input, Claude-style status line
**Date:** 2026-04-15
**Status:** Draft
**Scope:** `AgentFooter.tsx`, `AgentControlBar.tsx`, `agent-view.tsx`, `agent-view.scss`,
`claude-translator.ts`, `agent-model.ts` (or `useAgentStream.ts`), `types.ts`

---

## 1. What the user asked for

1. **Move the bypass/model/effort controls above the input box**, not below it.
2. **Move the loading indicator** to sit just below the textarea (its own line),
   styled like Claude Code's "Claude is thinking…" — not tucked in the hint bar.
3. **Show time + token/cost stats after a session finishes**, like Claude Code shows
   `$0.023 · 34s · 4 turns` at the end of a conversation.

---

## 2. Current layout (bottom of pane, top → bottom)

```
.agent-composer-region
├── AgentFooter
│   └── .agent-footer
│       └── .agent-input-container
│           ├── .slash-autocomplete   (absolute, above textarea)
│           ├── textarea.agent-input
│           └── .agent-input-hint    (9px, flex row)
│               ├── "Enter to send…" hint text
│               └── .agent-loading-spinner  (only while loading)
└── AgentControlBar
    └── .agent-control-bar           ← BELOW the input
        ├── collapsible header
        └── body: Mode · Model · Effort dropdowns
```

---

## 3. Target layout

```
.agent-composer-region
├── AgentControlBar                  ← MOVED ABOVE footer
│   └── .agent-control-bar
│       ├── collapsible header
│       └── body: Mode · Model · Effort dropdowns
└── AgentFooter
    └── .agent-footer
        └── .agent-input-container
            ├── .slash-autocomplete  (absolute, above textarea)
            ├── textarea.agent-input
            ├── .agent-status-line   ← NEW dedicated row (replaces loading in hint)
            │   • While loading:  ⏳ "Thinking…" (or current tool name)
            │   • After session:  "$0.023  ·  34s  ·  4 turns"
            │   • Otherwise:      empty / hidden (no layout shift)
            └── .agent-input-hint    (hint text only, no spinner)
```

---

## 4. Detailed behaviour spec

### 4.1 Control bar moved above input

- In `agent-view.tsx`, the `<AgentControlBar>` element that currently comes
  **after** `<AgentFooter>` in `.agent-composer-region` must be placed **before**
  it. No other changes to `AgentControlBar` logic.
- The top border on `.agent-control-bar` becomes a bottom border (or keep
  top-border as-is — it now separates the bar from the document, which is fine).
- No behaviour change — collapse/expand, non-default indicator (`*`), all
  existing interactions stay the same.

### 4.2 Loading indicator — dedicated status line

**Remove** `.agent-loading-spinner` from `.agent-input-hint`.

**Add** a new `.agent-status-line` div **below** the textarea, **above** the
hint line.

States:

| Condition | Display |
|-----------|---------|
| `isLoading && currentTool == null` | `⏳ Thinking…` (pulsing dot + text) |
| `isLoading && currentTool != null` | `⏳ <tool name>` (e.g. "⏳ Bash") |
| `!isLoading && sessionStats != null` | `$0.023  ·  34s  ·  4 turns` |
| `!isLoading && sessionStats == null` | hidden (`display: none` or `visibility: hidden`) |

The status line must not cause a layout shift when it appears/disappears.
Use `min-height: 14px` so the row is always reserved even when empty.

**`currentTool`**: the name of the last tool seen in a running `tool_call`
event (from `useAgentStream`). Reset to `null` on session end.

**Styling** (target — reference Claude Code's style):
```scss
.agent-status-line {
    min-height: 14px;
    font-size: 10px;
    color: secondary-text-color (60% opacity);
    padding: 1px 4px;
    display: flex;
    align-items: center;
    gap: 5px;
    font-family: var(--fixed-font, monospace);
}

// Loading variant
.agent-status-line--loading {
    color: #8cc8ff;
    .agent-spinner-dot { /* existing pulse animation */ }
}

// Stats variant (session just ended)
.agent-status-line--stats {
    color: secondary-text-color (50% opacity);
    gap: 0;  // use · separator with spacing in text
}
```

### 4.3 Session stats — time, cost, turns

**Data source:** The Claude Code CLI emits a `result` event as the final
stream-json line when a session ends:

```json
{
  "type": "result",
  "subtype": "success",
  "cost_usd": 0.023,
  "is_error": false,
  "duration_ms": 34211,
  "num_turns": 4,
  "session_id": "...",
  "total_input_tokens": 12345,
  "total_output_tokens": 678
}
```

**What to show:** `$0.023  ·  34s  ·  4 turns`

Rules:
- Cost: `$` prefix, 3 decimal places (e.g. `$0.023`). Hide if `cost_usd` is
  absent or 0 (Codex/Gemini don't emit cost).
- Duration: `duration_ms / 1000` rounded to nearest second (e.g. `34s`).
  If < 1s show `<1s`. If >= 60s show `1m 4s`.
- Turns: `num_turns` + ` turn` / ` turns` (pluralise). Hide if 0 or absent.
- Separator: `  ·  ` (two spaces, centered dot, two spaces).
- If only one field is available, show it alone (no orphan separators).

**Persistence:** Stats stay visible until the user starts the next message
(on textarea focus + keydown, or on send). They disappear when the next
session starts.

**Codex / Gemini:** Their translators don't emit a `result` event with cost
today. The stats line simply stays hidden — no placeholders, no zeros.

---

## 5. Data flow

### 5.1 New `SessionStats` type (add to `types.ts`)

```ts
export interface SessionStats {
    cost_usd?: number;         // from result.cost_usd
    duration_ms?: number;      // from result.duration_ms
    num_turns?: number;        // from result.num_turns
}
```

### 5.2 `claude-translator.ts` — handle `result` event

The translator currently ignores the top-level `result` event. Add a case:

```ts
// In the translate() method, Case X: top-level "result" event
if (rawEvent.type === "result") {
    const stats: SessionStats = {};
    if (typeof rawEvent.cost_usd === "number") stats.cost_usd = rawEvent.cost_usd;
    if (typeof rawEvent.duration_ms === "number") stats.duration_ms = rawEvent.duration_ms;
    if (typeof rawEvent.num_turns === "number") stats.num_turns = rawEvent.num_turns;
    return [{ type: "session_end", stats }];
}
```

Add `"session_end"` to `StreamEvent`:

```ts
// types.ts
export interface SessionEndEvent {
    type: "session_end";
    stats: SessionStats;
}
// Union: StreamEvent = TextEvent | ThinkingEvent | ToolCallEvent | ... | SessionEndEvent
```

### 5.3 `useAgentStream.ts` / `agent-model.ts` — store stats signal

Add to the model:

```ts
private _sessionStats = createSignal<SessionStats | null>(null);
sessionStatsAtom: Accessor<SessionStats | null> = this._sessionStats[0];
```

When a `session_end` event arrives:
- Set `sessionStats` to the event's stats.
- Set `isLoading` to `false`.
- Set `currentTool` to `null`.

When the user sends the next message:
- Clear `sessionStats` to `null`.

### 5.4 `AgentFooter.tsx` — render status line

```tsx
const statusLine = (): JSX.Element => {
    if (props.loading) {
        return (
            <span class="agent-status-line agent-status-line--loading">
                <span class="agent-spinner-dot" />
                {props.currentTool ? props.currentTool : "Thinking…"}
            </span>
        );
    }
    const stats = props.sessionStats;
    if (!stats) return <span class="agent-status-line" />;

    const parts: string[] = [];
    if (stats.cost_usd) parts.push(`$${stats.cost_usd.toFixed(3)}`);
    if (stats.duration_ms != null) {
        const s = Math.round(stats.duration_ms / 1000);
        parts.push(s < 60 ? `${Math.max(1, s)}s` : `${Math.floor(s / 60)}m ${s % 60}s`);
    }
    if (stats.num_turns) parts.push(`${stats.num_turns} ${stats.num_turns === 1 ? "turn" : "turns"}`);

    return (
        <span class="agent-status-line agent-status-line--stats">
            {parts.join("  ·  ")}
        </span>
    );
};
```

New props on `AgentFooterProps`:
- `currentTool?: string | null` — name of actively running tool
- `sessionStats?: SessionStats | null` — stats from the last completed session

---

## 6. Hint line cleanup

After moving the spinner out, `.agent-input-hint` contains only:

```
"Enter to send • Shift+Enter for newline • Esc to clear / stop"
```

The `justify-content: space-between` flex layout on `.agent-input-hint` can
be simplified to a single left-aligned span. Remove all loading-related
code from the hint rendering path.

---

## 7. Implementation steps (single PR)

| Step | Touch | Notes |
|------|-------|-------|
| 1. Swap order in `agent-view.tsx` | `agent-view.tsx` | Move `<AgentControlBar>` before `<AgentFooter>` in `.agent-composer-region` |
| 2. Add `SessionStats` + `SessionEndEvent` | `types.ts` | |
| 3. Handle `result` event | `claude-translator.ts` | Emit `session_end` event |
| 4. Store stats + currentTool | `agent-model.ts` / `useAgentStream.ts` | Add signals; clear on next send |
| 5. New status line in footer | `AgentFooter.tsx` | Replace hint spinner; add `currentTool` + `sessionStats` props |
| 6. SCSS | `agent-view.scss` | `.agent-status-line`, `--loading`, `--stats` variants; clean hint |
| 7. Wire props | `agent-view.tsx` (or wherever AgentFooter is instantiated) | Pass `currentTool` + `sessionStats` from model |

---

## 8. What this does NOT change

- `AgentControlBar` internals — only its position in the DOM.
- The `AgentControlBar` collapse/expand behaviour.
- Codex / Gemini stats (they stay hidden — no fake data).
- The `.agent-input-hint` text content.
- Any existing tool block display or document node rendering.

---

## 9. Reference: Claude Code's status appearance

After a Claude Code session ends, the CLI prints a line like:

```
Cost: $0.023 | Duration: 34s | Turns: 4
```

AgentMux's `.agent-status-line--stats` should feel similar: small, monospace,
dimmed, single-line, located immediately below the input box. Not a toast,
not a banner — just a quiet stats line that disappears on next send.
