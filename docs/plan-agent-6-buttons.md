# Plan: Agent Pane — 6 Provider Buttons (Raw + Styled)

## Goal

The agent pane shows 6 buttons in two rows:

```
[ Raw ]
  Claude (raw)   Gemini (raw)   Codex (raw)

[ Styled ]
  Claude (styled)  Gemini (styled)  Codex (styled)
```

- **Raw**: opens a plain PTY terminal running the CLI (existing behavior)
- **Styled**: stays in agent view, runs CLI with JSON streaming flags, renders output through the translator pipeline as a structured document

---

## Files to Change

### 1. `frontend/app/view/agent/providers/index.ts`

Add `styledArgs: string[]` and `styledOutputFormat` to `ProviderDefinition`:

```ts
styledArgs: string[];  // CLI flags for JSON streaming mode
styledOutputFormat: "claude-stream-json" | "gemini-json" | "codex-json";
```

Provider values:
- Claude: `styledArgs: ["--output-format", "stream-json", "--verbose"]`, format `"claude-stream-json"`
- Gemini: `styledArgs: ["--json"]`, format `"gemini-json"`  (verify exact flag)
- Codex: `styledArgs: ["--json"]`, format `"codex-json"`    (verify exact flag)

---

### 2. `frontend/app/view/agent/agent-model.ts`

Add `connectStyled(providerId, cliPath)`:
- Does NOT switch `view` to `"term"`
- Sets block meta: `{ agentMode: "styled", agentProvider: providerId, agentCliPath: cliPath }`
- Calls `ControllerResyncCommand` to notify backend

Keep existing `connectWithProvider` as `connectRaw`.

Expose both on the model:
```ts
connectRaw: (providerId: string, cliPath: string) => Promise<void>
connectStyled: (providerId: string, cliPath: string) => Promise<void>
```

---

### 3. `frontend/app/view/agent/agent-view.tsx`

**Picker screen changes:**

Replace single row of 3 buttons with two labelled rows:

```tsx
<div className="agent-mode-group">
  <div className="agent-mode-label">Raw</div>
  <div className="agent-provider-buttons">
    {providers.map(p => <ProviderButton key={p.id} provider={p} mode="raw" ... />)}
  </div>
</div>
<div className="agent-mode-group">
  <div className="agent-mode-label agent-mode-label--styled">Styled</div>
  <div className="agent-provider-buttons">
    {providers.map(p => <ProviderButton key={p.id} provider={p} mode="styled" ... />)}
  </div>
</div>
```

**ProviderButton** gets a `mode: "raw" | "styled"` prop:
- Raw button: existing monospace/border style
- Styled button: accent color tint, small "✦" or "◈" indicator

**Styled session screen (new):**

When block meta has `agentMode: "styled"`, render the document view instead of the picker:

```tsx
if (agentMode === "styled") {
  return <AgentStyledSession model={model} provider={agentProvider} />;
}
```

`AgentStyledSession`:
- Shows provider name + "Styled" badge in header area
- Renders `documentAtom` contents as `DocumentNode[]` (existing node components)
- Shows "Starting session..." if document is empty
- Has a status bar at bottom: connected/streaming indicator

---

### 4. `frontend/app/view/agent/agent-view.scss`

Add:
```scss
.agent-mode-group        // wraps label + buttons row
.agent-mode-label        // "Raw" / "Styled" section label
.agent-mode-label--styled  // accent color for styled label
.agent-provider-btn--styled  // styled variant: accent tint, border
.agent-styled-session    // full session view container
.agent-styled-header     // provider + badge
.agent-styled-status     // bottom status bar
```

---

## Streaming Pipeline (Styled Mode)

The full styled pipeline: CLI stdout → translator → documentAtom → render.

### How output gets from CLI to agent view

**Option A (preferred): Backend Tauri command**

Add `start_cli_session(provider, cli_path, styled_args) → session_id` Tauri command in a new `src-tauri/src/commands/cli_session.rs`:
- Spawns CLI process with JSON flags
- Reads stdout line by line
- Emits Tauri events: `cli-session-line:{blockId}` with raw NDJSON line
- Frontend listens via `listen()`, feeds each line through `createTranslator(outputFormat)`
- Translator output (`StreamEvent[]`) updates `documentAtom` via `applyStreamEvent()`

**Option B (fallback): Tauri shell plugin**

Use `@tauri-apps/plugin-shell` `Command.create()` to spawn the CLI directly from frontend.
- Blocked by: capabilities only allow sidecar spawns currently
- Fix: add external CLI names to `shell:allow-spawn` in `src-tauri/capabilities/default.json`
- Simpler but less control; process lifecycle tied to frontend

**Decision: Option A** — backend command gives us process lifecycle control, ability to inject env vars, and a clean event channel. Also avoids capability allowlist management for arbitrary user-installed binary paths.

### `applyStreamEvent(event, documentAtom)` helper (new file: `agent-session.ts`)

Maps `StreamEvent` → `DocumentNode` mutations:
- `text` → append to last `MarkdownNode` or create new one
- `thinking` → create `MarkdownNode` with `metadata.thinking = true`
- `tool_call` → create `ToolNode` with `status: "running"`
- `tool_result` → find matching `ToolNode` by id, update status/result
- `agent_message` → create `AgentMessageNode`
- `user_message` → create `UserMessageNode`

---

## Out of Scope (follow-up)

- Gemini/Codex styledArgs: need to verify exact JSON streaming flags from their CLIs
- Auth flow for styled mode (detect → prompt login if needed)
- Input box in styled session (send messages to running CLI)
- Session restart / kill controls
- Scroll-to-bottom, keyboard nav, filter bar
- Gemini + Codex translator implementations (currently stubs)

---

## Order of Implementation

1. `providers/index.ts` — add `styledArgs` + `styledOutputFormat`
2. `agent-model.ts` — add `connectRaw` / `connectStyled`
3. `agent-view.tsx` — 6 buttons UI + styled session scaffold
4. `agent-view.scss` — new classes
5. `cli_session.rs` (Tauri command) — spawn CLI, emit line events
6. `agent-session.ts` — `applyStreamEvent()` helper
7. Wire frontend: `listen()` → translator → `applyStreamEvent()` → atom
