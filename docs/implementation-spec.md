# AgentMux — Full Implementation Spec

**Author:** AgentX
**Date:** 2026-03-10
**Status:** Approved for implementation
**Base Version:** 0.31.100

---

## Table of Contents

1. [Phase 1 — Styled Agent View](#phase-1--styled-agent-view)
2. [Phase 2 — Wire Jekt](#phase-2--wire-jekt)
3. [Phase 3 — Forge Storage](#phase-3--forge-storage)
4. [Phase 4 — Forge UI](#phase-4--forge-ui)
5. [Phase 5 — JektRouter](#phase-5--jektrouter)
6. [Phase 6 — Forge Launch + Skills](#phase-6--forge-launch--skills)
7. [Phase 7 — Swarm](#phase-7--swarm)
8. [Phase 8 — Agent Output Stream](#phase-8--agent-output-stream)
9. [Phase 9 — OpenClaw](#phase-9--openclaw)

---

## Phase 1 — Styled Agent View

**Goal:** Working styled presentation layer for the agent pane. Bootstrap output visible during startup. Completed turns rendered from JSONL disk file. Input via PTY.

**Architecture decision:** Use the JSONL file (`~/.claude/projects/<cwd>/<session>.jsonl`) as the data source for completed turns rather than parsing streaming deltas from the PTY. The PTY remains the source for (a) bootstrap/init output before the agent produces JSON, and (b) all input injection.

```
Bootstrap phase (PTY only):
  PTY bytes → InitializationMonitor → TerminalOutputBlock (live streaming)

Agent running phase (JSONL + PTY):
  JSONL new line → parse content blocks → DocumentNode → render
  PTY input → RpcApi.ControllerInputCommand → agent receives

```

### 1.1 Backend — JSONL File Watcher

**Files:**
- `agentmuxsrv-rs/src/backend/agentwatch/mod.rs` `[NEW]`
- `agentmuxsrv-rs/src/backend/agentwatch/watcher.rs` `[NEW]`
- `agentmuxsrv-rs/src/backend/mod.rs` `[MOD]` — add `pub mod agentwatch;`
- `agentmuxsrv-rs/src/server/mod.rs` `[MOD]` — add SSE route (also used in Phase 8)

**Goal:** Watch a Claude session JSONL file and broadcast new lines to the frontend via the existing WebSocket event bus.

**`agentmuxsrv-rs/src/backend/agentwatch/watcher.rs`**

```rust
use notify::{Watcher, RecursiveMode, Event, EventKind};
use std::path::{Path, PathBuf};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::fs::File;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;

/// Tracks file position per watched session file.
pub struct SessionWatcher {
    positions: Arc<Mutex<HashMap<PathBuf, u64>>>,
    _watcher: notify::RecommendedWatcher,
}

pub type LineCallback = Arc<dyn Fn(String, String) + Send + Sync>;
// Args: (session_file_path_str, jsonl_line)

impl SessionWatcher {
    pub fn new(callback: LineCallback) -> notify::Result<Self> {
        let positions: Arc<Mutex<HashMap<PathBuf, u64>>> = Arc::new(Mutex::new(HashMap::new()));
        let positions_clone = positions.clone();
        let cb = callback.clone();

        let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
            if let Ok(event) = res {
                if matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                    for path in &event.paths {
                        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                            continue;
                        }
                        read_new_lines(path, &positions_clone, &cb);
                    }
                }
            }
        })?;

        Ok(Self {
            positions,
            _watcher: watcher,
        })
    }

    /// Start watching a specific session JSONL file.
    pub fn watch_session(&mut self, path: &Path) -> notify::Result<()> {
        // Record current file size as starting position (don't replay history)
        let pos = std::fs::metadata(path)
            .map(|m| m.len())
            .unwrap_or(0);
        self.positions.lock().unwrap().insert(path.to_path_buf(), pos);
        self._watcher.watch(path.parent().unwrap_or(path), RecursiveMode::NonRecursive)?;
        Ok(())
    }

    /// Stop watching a session file.
    pub fn unwatch_session(&mut self, path: &Path) {
        self.positions.lock().unwrap().remove(path);
        let _ = self._watcher.unwatch(path.parent().unwrap_or(path));
    }
}

fn read_new_lines(
    path: &Path,
    positions: &Arc<Mutex<HashMap<PathBuf, u64>>>,
    callback: &LineCallback,
) {
    let mut pos_map = positions.lock().unwrap();
    let pos = pos_map.entry(path.to_path_buf()).or_insert(0);

    let Ok(mut file) = File::open(path) else { return };
    let Ok(_) = file.seek(SeekFrom::Start(*pos)) else { return };

    let mut reader = BufReader::new(&file);
    let mut line = String::new();
    let path_str = path.to_string_lossy().to_string();

    while reader.read_line(&mut line).unwrap_or(0) > 0 {
        let trimmed = line.trim_end().to_string();
        if !trimmed.is_empty() {
            callback(path_str.clone(), trimmed);
        }
        line.clear();
    }

    *pos = file.seek(SeekFrom::Current(0)).unwrap_or(*pos);
}
```

**`agentmuxsrv-rs/src/backend/agentwatch/mod.rs`**

```rust
pub mod watcher;
pub use watcher::SessionWatcher;

/// Resolve the Claude session directory for a given working directory.
/// Claude encodes the cwd path by replacing non-alphanumeric chars with '-'.
pub fn claude_session_dir(cwd: &str) -> std::path::PathBuf {
    let home = dirs::home_dir().unwrap_or_default();
    let encoded: String = cwd.chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    home.join(".claude").join("projects").join(encoded)
}

/// Find the most recent session JSONL file for a given cwd.
pub fn latest_session_file(cwd: &str) -> Option<std::path::PathBuf> {
    let dir = claude_session_dir(cwd);
    std::fs::read_dir(&dir).ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("jsonl"))
        .max_by_key(|e| e.metadata().ok().and_then(|m| m.modified().ok()))
        .map(|e| e.path())
}
```

**Route addition** (in `server/mod.rs`) — adds two endpoints:
- `POST /api/agentwatch/watch` — `{block_id, cwd}` → start watching most recent session file for that cwd
- `POST /api/agentwatch/unwatch` — `{block_id}` → stop watching

When a new JSONL line arrives, emit a WebSocket event to the frontend:
```json
{
  "type": "agentwatch:line",
  "block_id": "<block-id>",
  "line": "<raw-jsonl-line>"
}
```

This reuses the existing WebSocket event bus — same pattern as `FileUpdate` events.

### 1.2 Frontend — JSONL Line Types

**`frontend/app/view/agent/jsonl-types.ts`** `[NEW]`

The JSONL format written by Claude Code. Each line is one of:

```typescript
// User turn
export interface JsonlUserMessage {
    type: "user";
    message: { role: "user"; content: string | ContentBlock[] };
    uuid: string;
    timestamp: string;
    cwd: string;
    gitBranch?: string;
    sessionId?: string;
}

// Assistant turn (complete — written when turn finishes)
export interface JsonlAssistantMessage {
    type: "assistant";
    message: {
        role: "assistant";
        content: ContentBlock[];
        usage?: { input_tokens: number; output_tokens: number };
    };
    uuid: string;
    timestamp: string;
    sessionId?: string;
}

export type ContentBlock =
    | { type: "text"; text: string }
    | { type: "thinking"; thinking: string; signature?: string }
    | { type: "tool_use"; id: string; name: string; input: Record<string, unknown> }
    | { type: "tool_result"; tool_use_id: string; content: string | ContentBlock[]; is_error?: boolean };

export type JsonlLine = JsonlUserMessage | JsonlAssistantMessage;
```

### 1.3 Frontend — JSONL → DocumentNode Mapper

**`frontend/app/view/agent/jsonl-mapper.ts`** `[NEW]`

Maps parsed JSONL lines directly to `DocumentNode` objects. Replaces the translator + stream-parser pipeline for the read path.

```typescript
import type { JsonlLine, ContentBlock } from "./jsonl-types";
import type { DocumentNode, MarkdownNode, ToolNode, UserMessageNode } from "./types";

let nodeCounter = 0;
const nextId = () => `jsonl_${++nodeCounter}_${Date.now()}`;

export function jsonlLineToNodes(line: JsonlLine): DocumentNode[] {
    const nodes: DocumentNode[] = [];

    if (line.type === "user") {
        const content = typeof line.message.content === "string"
            ? line.message.content
            : line.message.content
                .filter((b): b is { type: "text"; text: string } => b.type === "text")
                .map(b => b.text).join("");

        if (content.trim()) {
            nodes.push({
                type: "user_message",
                id: `user_${line.uuid}`,
                message: content,
                timestamp: new Date(line.timestamp).getTime(),
                collapsed: false,
                summary: "You",
            } satisfies UserMessageNode);
        }
        return nodes;
    }

    if (line.type === "assistant") {
        // Build a tool_use → tool_result map for pairing
        const toolResults = new Map<string, ContentBlock & { type: "tool_result" }>();
        for (const block of line.message.content) {
            if (block.type === "tool_result") {
                toolResults.set(block.tool_use_id, block);
            }
        }

        for (const block of line.message.content) {
            if (block.type === "text" && block.text.trim()) {
                nodes.push({
                    type: "markdown",
                    id: nextId(),
                    content: block.text,
                    collapsed: false,
                    summary: block.text.slice(0, 80),
                    metadata: {},
                } satisfies MarkdownNode);
            }

            if (block.type === "thinking" && block.thinking.trim()) {
                nodes.push({
                    type: "markdown",
                    id: nextId(),
                    content: block.thinking,
                    collapsed: true,
                    summary: "Thinking...",
                    metadata: { thinking: true },
                } satisfies MarkdownNode);
            }

            if (block.type === "tool_use") {
                const result = toolResults.get(block.id);
                const resultContent = result
                    ? (typeof result.content === "string"
                        ? result.content
                        : result.content.map(b => "text" in b ? b.text : "").join(""))
                    : null;

                nodes.push({
                    type: "tool",
                    id: `tool_${block.id}`,
                    tool: block.name,
                    params: block.input,
                    status: result ? (result.is_error ? "error" : "success") : "running",
                    result: resultContent ?? undefined,
                    collapsed: result && !result.is_error,
                    summary: `${block.name}(${summarizeParams(block.input)})`,
                } satisfies ToolNode);
            }
        }
        return nodes;
    }

    return nodes;
}

function summarizeParams(input: Record<string, unknown>): string {
    const entries = Object.entries(input);
    if (entries.length === 0) return "";
    const [key, val] = entries[0];
    const valStr = typeof val === "string" ? val.slice(0, 40) : JSON.stringify(val).slice(0, 40);
    return entries.length > 1 ? `${key}: "${valStr}", ...` : `${key}: "${valStr}"`;
}
```

### 1.4 Frontend — TerminalOutputBlock Component

Surfaces raw PTY output during bootstrap (before the agent starts producing JSON). This is the "Starting..." phase — npm install, auth flows, CLI startup messages.

**`frontend/app/view/agent/components/TerminalOutputBlock.tsx`** `[NEW]`

```tsx
import React, { memo } from "react";
import type { TerminalOutputNode } from "../types";

export const TerminalOutputBlock = memo(({ node }: { node: TerminalOutputNode }) => {
    return (
        <div className={`agent-terminal-output ${node.complete ? "complete" : "active"}`}>
            <div className="agent-terminal-header">
                <span className="agent-terminal-icon">{node.complete ? "✓" : "⏳"}</span>
                <span className="agent-terminal-label">
                    {node.complete ? "Bootstrap" : "Starting..."}
                </span>
            </div>
            <pre className="agent-terminal-content">{node.content}</pre>
        </div>
    );
});
```

Add `TerminalOutputNode` to `types.ts`:

```typescript
export interface TerminalOutputNode {
    type: "terminal_output";
    id: string;
    content: string;
    complete: boolean;  // true once first JSON line arrives
    collapsed: boolean;
    summary: string;
}
```

Update `DocumentNode` union to include `TerminalOutputNode`.

### 1.5 Frontend — useAgentStream Hook (revised)

**`frontend/app/view/agent/useAgentStream.ts`** `[NEW]`

Two subscriptions in one hook:

1. **PTY subscription** — for bootstrap output (non-JSON lines before first valid JSONL line)
2. **WebSocket `agentwatch:line` event** — for completed JSONL turns

```typescript
import { useEffect, useRef } from "react";
import { useSetAtom } from "jotai";
import { getFileSubject, base64ToArray } from "@/app/store/wps";
import { globalStore } from "@/app/store/global";
import { getEventBusSubject } from "@/app/store/ws";
import { jsonlLineToNodes } from "./jsonl-mapper";
import type { AgentAtoms } from "./state";
import type { TerminalOutputNode, DocumentNode } from "./types";

interface UseAgentStreamOpts {
    blockId: string;
    cwd: string;
    atoms: AgentAtoms;
    enabled: boolean;
}

export function useAgentStream({ blockId, cwd, atoms, enabled }: UseAgentStreamOpts) {
    const jsonStartedRef = useRef(false);
    const terminalNodeIdRef = useRef<string | null>(null);
    const decoderRef = useRef(new TextDecoder());
    const bufferRef = useRef("");

    useEffect(() => {
        if (!enabled || !blockId) return;

        jsonStartedRef.current = false;
        terminalNodeIdRef.current = null;
        bufferRef.current = "";

        // ── PTY subscription (bootstrap output) ──────────────────────────
        const fileSubject = getFileSubject(blockId, "term");

        const ptySub = fileSubject.subscribe((msg) => {
            if (msg.fileop === "truncate") {
                globalStore.set(atoms.documentAtom, []);
                jsonStartedRef.current = false;
                terminalNodeIdRef.current = null;
                bufferRef.current = "";
                return;
            }
            if (msg.fileop !== "append" || jsonStartedRef.current) return;

            const bytes = base64ToArray(msg.data64);
            const text = decoderRef.current.decode(bytes, { stream: true });
            bufferRef.current += text;
            const lines = bufferRef.current.split("\n");
            bufferRef.current = lines.pop() ?? "";

            for (const line of lines) {
                const trimmed = line.trim();
                if (!trimmed) continue;

                // Check if this is valid JSON (agent has started)
                try {
                    JSON.parse(trimmed);
                    // First valid JSON line — mark terminal node complete
                    jsonStartedRef.current = true;
                    if (terminalNodeIdRef.current) {
                        const doc = globalStore.get(atoms.documentAtom);
                        const idx = doc.findIndex(n => n.id === terminalNodeIdRef.current);
                        if (idx >= 0) {
                            const updated = [...doc];
                            (updated[idx] as TerminalOutputNode).complete = true;
                            (updated[idx] as TerminalOutputNode).collapsed = true;
                            (updated[idx] as TerminalOutputNode).summary = "Bootstrap ✓";
                            globalStore.set(atoms.documentAtom, updated);
                        }
                    }
                    break;
                } catch {
                    // Non-JSON — bootstrap output, accumulate into terminal node
                    const nodeId = terminalNodeIdRef.current;
                    if (!nodeId) {
                        // Create new terminal output node
                        const newId = `term_${blockId}_${Date.now()}`;
                        terminalNodeIdRef.current = newId;
                        const node: TerminalOutputNode = {
                            type: "terminal_output",
                            id: newId,
                            content: trimmed + "\n",
                            complete: false,
                            collapsed: false,
                            summary: "Starting...",
                        };
                        globalStore.set(atoms.appendNodeAtom, node);
                    } else {
                        // Append to existing terminal node
                        const doc = globalStore.get(atoms.documentAtom);
                        const idx = doc.findIndex(n => n.id === nodeId);
                        if (idx >= 0) {
                            const updated = [...doc];
                            (updated[idx] as TerminalOutputNode).content += trimmed + "\n";
                            globalStore.set(atoms.documentAtom, updated);
                        }
                    }
                }
            }
        });

        // ── agentwatch:line subscription (completed turns) ────────────────
        // Tell backend to start watching the session file
        fetch("/api/agentwatch/watch", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ block_id: blockId, cwd }),
        }).catch(console.error);

        const eventBus = getEventBusSubject();
        const jsonlSub = eventBus.subscribe((event) => {
            if (event.type !== "agentwatch:line") return;
            if (event.block_id !== blockId) return;

            try {
                const parsed = JSON.parse(event.line);
                const nodes = jsonlLineToNodes(parsed);
                for (const node of nodes) {
                    globalStore.set(atoms.appendNodeAtom, node);
                }
            } catch {
                // Malformed line — ignore
            }
        });

        return () => {
            ptySub.unsubscribe?.();
            fileSubject.release();
            jsonlSub.unsubscribe?.();
            fetch("/api/agentwatch/unwatch", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({ block_id: blockId }),
            }).catch(() => {});
        };
    }, [blockId, cwd, enabled]);
}
```

### 1.6 Frontend — AgentDocumentView Component

**`frontend/app/view/agent/components/AgentDocumentView.tsx`** `[NEW]`

```tsx
import React, { memo, useRef, useEffect } from "react";
import { useAtomValue } from "jotai";
import { MarkdownBlock } from "./MarkdownBlock";
import { ToolBlock } from "./ToolBlock";
import { AgentMessageBlock } from "./AgentMessageBlock";
import { TerminalOutputBlock } from "./TerminalOutputBlock";
import type { AgentAtoms } from "../state";
import type { DocumentNode } from "../types";

interface Props { atoms: AgentAtoms }

export const AgentDocumentView = memo(({ atoms }: Props) => {
    const document = useAtomValue(atoms.documentAtom);
    const filters = useAtomValue(atoms.filterAtom);
    const scrollRef = useRef<HTMLDivElement>(null);
    const nodeIdSetRef = useRef(new Set<string>());

    // Auto-scroll when new nodes arrive
    useEffect(() => {
        const el = scrollRef.current;
        if (!el) return;
        const isNearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 120;
        if (isNearBottom) el.scrollTop = el.scrollHeight;
    }, [document.length]);

    // Dedup by id
    const seen = new Set<string>();
    const filtered = document.filter((node) => {
        if (seen.has(node.id)) return false;
        seen.add(node.id);
        if (node.type === "markdown" && node.metadata?.thinking && !filters.showThinking) return false;
        if (node.type === "tool" && !filters.showTools) return false;
        return true;
    });

    return (
        <div className="agent-document-view" ref={scrollRef}>
            {filtered.length === 0 && (
                <div className="agent-document-empty">Waiting for output...</div>
            )}
            {filtered.map(node => <NodeRenderer key={node.id} node={node} atoms={atoms} />)}
        </div>
    );
});

const NodeRenderer = memo(({ node, atoms }: { node: DocumentNode; atoms: AgentAtoms }) => {
    switch (node.type) {
        case "terminal_output": return <TerminalOutputBlock node={node} />;
        case "markdown":        return <MarkdownBlock node={node} />;
        case "tool":            return <ToolBlock node={node} atoms={atoms} />;
        case "agent_message":   return <AgentMessageBlock node={node} />;
        case "user_message":    return <UserMessageBlock node={node} />;
        default:                return null;
    }
});

const UserMessageBlock = memo(({ node }: { node: any }) => (
    <div className="agent-user-message">
        <div className="agent-user-message-label">You</div>
        <div className="agent-user-message-body">{node.message}</div>
    </div>
));
```

### 1.7 Frontend — Wire into agent-view.tsx

**`frontend/app/view/agent/agent-view.tsx`** `[MOD]`

Replace the `AgentStyledSession` placeholder with:

```tsx
function AgentStyledSession({ blockId, model }: { blockId: string; model: AgentViewModel }) {
    const [blockData] = useWaveObjectValue<Block>(makeORef("block", blockId));
    const providerId = blockData?.meta?.["agentProvider"] ?? "claude";
    const cwd = blockData?.meta?.["cmd:cwd"] ?? "";
    const atoms = useMemo(() => createAgentAtoms(blockId), [blockId]);

    useAgentStream({ blockId, cwd, atoms, enabled: true });

    const handleSend = useCallback((text: string) => {
        if (!text.trim()) return;
        RpcApi.ControllerInputCommand(TabRpcClient, {
            blockid: blockId,
            inputdata64: stringToBase64(text + "\n"),
        });
    }, [blockId]);

    return (
        <div className="agent-styled-session">
            <AgentHeader blockId={blockId} provider={getProvider(providerId)} atoms={atoms}
                onDisconnect={() => model.disconnectStyled()} />
            <FilterControls atoms={atoms} />
            <AgentDocumentView atoms={atoms} />
            <AgentFooter onSendMessage={handleSend} />
        </div>
    );
}
```

### 1.8 SCSS

**`frontend/app/view/agent/agent-view.scss`** `[MOD]`

```scss
.agent-styled-session {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow: hidden;
}

.agent-document-view {
    flex: 1;
    overflow-y: auto;
    padding: 8px 12px;
}

.agent-document-empty {
    color: var(--secondary-text-color);
    text-align: center;
    padding: 40px;
    font-style: italic;
}

.agent-user-message {
    margin: 8px 0;
    padding: 8px 12px;
    border-left: 3px solid var(--accent-color);
    background: color-mix(in srgb, var(--accent-color) 5%, transparent);
    border-radius: 4px;

    .agent-user-message-label {
        font-size: 11px;
        color: var(--secondary-text-color);
        margin-bottom: 4px;
        font-weight: 600;
    }

    .agent-user-message-body {
        white-space: pre-wrap;
        font-size: 13px;
    }
}

.agent-terminal-output {
    border: 1px solid var(--border-color);
    border-radius: 4px;
    border-left: 3px solid var(--warning-color);
    background: color-mix(in srgb, var(--main-text-color) 3%, transparent);
    margin: 8px 0;

    &.complete {
        border-left-color: var(--success-color);
        opacity: 0.7;
    }

    .agent-terminal-header {
        display: flex;
        align-items: center;
        gap: 8px;
        padding: 6px 12px;
        font-size: 11px;
        color: var(--secondary-text-color);
        border-bottom: 1px solid var(--border-color);
    }

    .agent-terminal-content {
        padding: 8px 12px;
        margin: 0;
        font-size: 12px;
        line-height: 1.4;
        white-space: pre-wrap;
        word-break: break-word;
        max-height: 300px;
        overflow-y: auto;
    }
}
```

### 1.9 State Additions

**`frontend/app/view/agent/state.ts`** `[MOD]`

Add if not present:

```typescript
const appendNodeAtom = atom(null, (get, set, node: DocumentNode) => {
    const doc = get(documentAtom);
    set(documentAtom, [...doc, node]);
});

const updateNodeAtom = atom(null, (get, set, node: DocumentNode) => {
    const doc = get(documentAtom);
    const idx = doc.findIndex(n => n.id === node.id);
    if (idx >= 0) {
        const updated = [...doc];
        updated[idx] = node;
        set(documentAtom, updated);
    } else {
        set(documentAtom, [...doc, node]);
    }
});
```

### 1.10 Verification

```
1. task dev
2. Open agent pane → select Claude → Styled mode
3. Bootstrap: npm install / CLI startup lines appear in TerminalOutputBlock with ⏳
4. Once Claude starts: TerminalOutputBlock shows "Bootstrap ✓", collapses
5. User message → appears immediately as user_message node
6. Claude response → text renders as MarkdownBlock
7. Tool call → ToolBlock with correct name/params, success collapses
8. Filter: toggle Thinking off → thinking blocks hidden
9. Navigate away → navigate back → session document rebuilt from JSONL
```

---

## Phase 2 — Wire Jekt

**Goal:** Connect `reactive::handler` to `blockcontroller::send_input` so PTY injection works end-to-end. One file, one change.

**`agentmuxsrv-rs/src/main.rs`** `[MOD]`

Add import after existing `use backend::...` imports:

```rust
use backend::blockcontroller;
```

Add after `let reactive_handler = reactive::get_global_handler();`:

```rust
// Wire reactive handler to block controller for terminal injection (jekt)
reactive_handler.set_input_sender(Arc::new(|block_id: &str, data: &[u8]| {
    blockcontroller::send_input(
        block_id,
        blockcontroller::BlockInputUnion::data(data.to_vec()),
    )
}));
```

### 2.1 Enter Key Strategy

Use single payload — message + carriage return in one write:

```rust
// In reactive/handler.rs inject_message():
let payload = format!("{}\r", final_msg);
sender(&block_id, payload.as_bytes())?;
```

Test in bash, zsh, and pwsh. If Enter doesn't register in all shells, fall back to a tokio channel-based approach with a brief async delay between payload and `\r`.

### 2.2 Verification

```bash
# 1. Start dev mode
task dev

# 2. Open a terminal pane, note its block_id

# 3. Register the agent
curl -X POST http://localhost:<port>/wave/reactive/register \
  -H "Content-Type: application/json" \
  -d '{"agent_id": "test-agent", "block_id": "<block-id>"}'

# 4. Inject a message
curl -X POST http://localhost:<port>/wave/reactive/inject \
  -H "Content-Type: application/json" \
  -d '{"target_agent": "test-agent", "message": "echo hello from jekt"}'

# 5. Verify "echo hello from jekt" appears in the terminal and runs
```

---

## Phase 3 — Forge Storage

**Goal:** Persist agent configurations in SQLite. Backend-only — no UI yet.

### 3.1 New Files

| File | Purpose |
|------|---------|
| `agentmuxsrv-rs/src/backend/forge/mod.rs` | Module root, re-exports |
| `agentmuxsrv-rs/src/backend/forge/types.rs` | Data types |
| `agentmuxsrv-rs/src/backend/forge/storage.rs` | SQLite CRUD |
| `agentmuxsrv-rs/src/server/forge.rs` | HTTP handlers |

### 3.2 Modified Files

| File | Change |
|------|--------|
| `agentmuxsrv-rs/src/backend/mod.rs` | Add `pub mod forge;` |
| `agentmuxsrv-rs/src/backend/storage/wstore.rs` | Call `forge::storage::migrate(&conn)` |
| `agentmuxsrv-rs/src/server/mod.rs` | Add forge routes |

### 3.3 SQLite Schema

```sql
CREATE TABLE IF NOT EXISTS forge_agents (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    provider_id TEXT NOT NULL,
    working_directory TEXT NOT NULL DEFAULT '',
    shell TEXT NOT NULL DEFAULT 'bash',
    provider_flags TEXT NOT NULL DEFAULT '[]',   -- JSON array
    env_vars TEXT NOT NULL DEFAULT '{}',          -- JSON object
    auto_start INTEGER NOT NULL DEFAULT 0,
    restart_on_crash INTEGER NOT NULL DEFAULT 0,
    idle_timeout_minutes INTEGER NOT NULL DEFAULT 0,
    tags TEXT NOT NULL DEFAULT '[]',              -- JSON array
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    last_launched_at INTEGER
);

CREATE TABLE IF NOT EXISTS forge_content (
    agent_id TEXT NOT NULL,
    content_type TEXT NOT NULL,   -- agentmd | soul | skills | mcp | env
    content TEXT NOT NULL DEFAULT '',
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (agent_id, content_type),
    FOREIGN KEY (agent_id) REFERENCES forge_agents(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS forge_skills (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    trigger TEXT NOT NULL,
    skill_type TEXT NOT NULL,    -- command | prompt | workflow | mcp
    config TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS forge_agent_skills (
    agent_id TEXT NOT NULL,
    skill_id TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (agent_id, skill_id),
    FOREIGN KEY (agent_id) REFERENCES forge_agents(id) ON DELETE CASCADE,
    FOREIGN KEY (skill_id) REFERENCES forge_skills(id) ON DELETE CASCADE
);
```

### 3.4 HTTP Routes

```
GET    /api/forge/agents                          → list all agents
POST   /api/forge/agents                          → create agent
GET    /api/forge/agents/:id                      → get agent
PATCH  /api/forge/agents/:id                      → update agent
DELETE /api/forge/agents/:id                      → delete agent
GET    /api/forge/agents/:id/content/:type        → get content (agentmd|soul|skills|mcp|env)
PUT    /api/forge/agents/:id/content/:type        → set content
POST   /api/forge/agents/:id/edit/:type           → open in external editor
GET    /api/forge/skills                          → list global skills
POST   /api/forge/skills                          → create skill
DELETE /api/forge/skills/:id                      → delete skill
```

### 3.5 File Watcher for Content Sync

When the user edits an agent's content file externally, the changes must sync back to SQLite. Use the `notify` crate (already a dependency via Phase 1):

Watch `~/.wave/data/agents/{agent_id}/` for each agent. On file modify:
- Map filename → content type: `CLAUDE.md` → `agentmd`, `soul.md` → `soul`, `skills.json` → `skills`, `mcp.json` → `mcp`
- Read file content → `forge::storage::set_content()`
- Broadcast `forge:content-changed` event via WebSocket

### 3.6 Verification

```bash
# Create agent
curl -X POST http://localhost:<port>/api/forge/agents \
  -H "Content-Type: application/json" \
  -d '{"name": "test-agent", "provider_id": "claude", "working_directory": "/tmp"}'

# Set AgentMD content
curl -X PUT http://localhost:<port>/api/forge/agents/<id>/content/agentmd \
  -H "Content-Type: application/json" \
  -d '{"content": "# Test Agent\n\nYou are a helpful assistant."}'

# Retrieve it
curl http://localhost:<port>/api/forge/agents/<id>/content/agentmd

# Delete
curl -X DELETE http://localhost:<port>/api/forge/agents/<id>
```

---

## Phase 4 — Forge UI

**Goal:** Functional Forge pane in AgentMux for managing agent configs, editing content, and viewing skills.

### 4.1 New Files

```
frontend/app/view/forge/
├── forge-view.tsx          Main two-panel layout
├── agent-list.tsx          Left sidebar: agent list + create button
├── agent-detail.tsx        Right panel: tabs for AgentMD/Soul/Skills/MCP/Settings
├── content-preview.tsx     Read-only markdown/JSON preview + "Edit" button
├── skills-panel.tsx        Skills list with trigger/type/description
├── forge-model.ts          ViewModel + Jotai atoms
├── api.ts                  Backend API client
├── types.ts                TypeScript types (mirrors Rust types)
├── forge.scss              Styles
└── index.ts                Exports
```

### 4.2 Layout

```
┌─────────────────────────────────────────────────────────┐
│  The Forge                                          [+]  │
├───────────────────┬─────────────────────────────────────┤
│  Agents           │  my-agent                    [Launch]│
│  ─────────────    │  ──────────────────────────────────  │
│  ● my-agent       │  [AgentMD] [Soul] [Skills] [Settings]│
│  ○ reviewer       │                                      │
│  ○ fixer          │  # My Agent                          │
│                   │                                      │
│                   │  You are a helpful assistant...      │
│                   │                                      │
│                   │                       [✎ Edit]       │
└───────────────────┴─────────────────────────────────────┘
```

### 4.3 Content Tabs

| Tab | Content Type | Edit Opens |
|-----|-------------|------------|
| AgentMD | `agentmd` | `CLAUDE.md` / `GEMINI.md` / `AGENT.md` in external editor |
| Soul | `soul` | `soul.md` in external editor |
| Skills | `skills` | `skills.json` in external editor — rendered as skill cards |
| MCP | `mcp` | `.mcp.json` in external editor |
| Settings | n/a | Inline form: provider, working_directory, shell, flags, env_vars, auto_start, etc. |

**Content is read-only in the UI.** The "Edit" button calls `POST /api/forge/agents/:id/edit/:type`, which writes the current content to a temp file and opens it in `$VISUAL` / `$EDITOR` / system default. The file watcher detects the save and syncs back to SQLite. Frontend subscribes to `forge:content-changed` WebSocket events to refresh.

### 4.4 Block View Registration

Register `"forge"` as a new block view type. In the view router (wherever `"agent"`, `"term"` etc. are mapped):

```typescript
case "forge":
    return <ForgeView />;
```

Open Forge pane via: new block with `{view: "forge"}` in meta.

### 4.5 Verification

```
1. task dev
2. Open new pane → select Forge view type
3. Click [+] → create agent "test" with provider Claude
4. Select agent → AgentMD tab → click Edit → file opens in VS Code
5. Type content, save file
6. Return to AgentMux → AgentMD tab shows updated content
7. Skills tab → create skills.json → shows skill cards
8. Settings tab → change working_directory → save → persists across restart
```

---

## Phase 5 — JektRouter

**Goal:** Replace the current dual `ReactiveHandler`/`MessageBus` inject paths with a single unified `JektRouter`. Three tiers: Local (PTY), LAN (mDNS), Cloud (WebSocket relay).

### 5.1 New Files

| File | Purpose |
|------|---------|
| `agentmuxsrv-rs/src/backend/jekt/mod.rs` | Module root |
| `agentmuxsrv-rs/src/backend/jekt/router.rs` | JektRouter struct — registration + routing |
| `agentmuxsrv-rs/src/backend/jekt/types.rs` | JektRequest, JektResponse, AgentRegistration, JektTier |
| `agentmuxsrv-rs/src/backend/jekt/lan.rs` | LAN peer discovery via mDNS (Phase 5 LAN sub-phase) |
| `agentmuxsrv-rs/src/backend/jekt/cloud.rs` | Cloud relay WebSocket client (Phase 5 Cloud sub-phase) |
| `agentmuxsrv-rs/src/server/jekt.rs` | HTTP handlers |

### 5.2 JektRouter Core

```rust
pub struct JektRouter {
    inner: Mutex<JektRouterInner>,
}

struct JektRouterInner {
    // agent_id → block_id
    local_agents: HashMap<String, AgentRegistration>,
    agent_to_block: HashMap<String, String>,
    block_to_agent: HashMap<String, String>,
}

impl JektRouter {
    pub fn register(&self, agent_id: &str, block_id: &str, tab_id: Option<&str>)
    pub fn unregister(&self, agent_id: &str)
    pub fn jekt(&self, req: JektRequest) -> JektResponse
    pub fn list_agents(&self) -> Vec<AgentRegistration>
}
```

**Routing logic in `jekt()`:**

1. Validate agent_id (alphanumeric + hyphens/underscores only, max 64 chars)
2. Sanitize message (strip dangerous control chars, max 8192 chars)
3. Format message (prepend source agent info if present)
4. **Tier 1 — Local:** look up `agent_to_block`, call `blockcontroller::send_input(block_id, format!("{}\r", msg).as_bytes())`
5. **Tier 2 — LAN:** if not found locally, check `LanDiscovery.list_peers()`, HTTP POST to peer's `/api/jekt/inject`
6. **Tier 3 — Cloud:** if not on LAN, relay via cloud WebSocket (Phase 5 cloud sub-phase)
7. Return `JektResponse { success, tier, block_id, error }`

### 5.3 HTTP API

```
POST /api/jekt/inject          {target_agent, message, source_agent?, priority?}
POST /api/jekt/register        {agent_id, block_id, tab_id?}
DELETE /api/jekt/register/:id  → unregister
GET  /api/jekt/agents          → list registered agents
GET  /api/jekt/peers           → list LAN peers (Phase 5 LAN)
```

### 5.4 Backward Compatibility

Existing `/wave/reactive/inject` and `/wave/reactive/register` endpoints delegate to JektRouter. No breaking changes.

WebSocket `bus:inject` messages also route through JektRouter.

`AGENTMUX_AGENT_ID` env var is set at block launch (already in Forge Phase 6). Agents use this to identify themselves when calling `/api/jekt/inject`.

### 5.5 LAN Sub-Phase

**Dependencies:** `mdns-sd = "0.11"`, `reqwest = { version = "0.12", features = ["json"] }`

Service advertisement: `_agentmux._tcp.local.` with TXT records: `instance_id`, `version`.

Trust model: peers must be explicitly trusted via `POST /api/jekt/peers/:id/trust` before messages route to them. Default: discovered but untrusted.

### 5.6 Verification

```bash
# Register two agents (simulate two panes)
curl -X POST .../api/jekt/register -d '{"agent_id": "agent-a", "block_id": "<block-a>"}'
curl -X POST .../api/jekt/register -d '{"agent_id": "agent-b", "block_id": "<block-b>"}'

# Jekt from agent-a to agent-b
curl -X POST .../api/jekt/inject \
  -d '{"target_agent": "agent-b", "message": "hello from agent-a", "source_agent": "agent-a"}'

# Verify message appears in agent-b's terminal
```

---

## Phase 6 — Forge Launch + Skills

**Goal:** Launch a fully-configured agent from the Forge UI into a terminal pane, with all config files written to disk.

### 6.1 Launch Flow

`POST /api/forge/agents/:id/launch` handler in `server/forge.rs`:

1. Load `AgentConfig` + all `forge_content` rows from SQLite
2. Resolve working directory (default: `$HOME`)
3. `std::fs::create_dir_all(&working_dir)`
4. **Write AgentMD**: prepend Soul (if non-empty) + `\n\n---\n\n` + AgentMD content → write to `<working_dir>/CLAUDE.md` (or `GEMINI.md`, `AGENT.md` per provider)
5. **Write MCP config**: if `mcp` content non-empty → write `<working_dir>/.mcp.json`
6. **Build block metadata**:
   ```
   view = "term"
   controller = "cmd"
   cmd = <provider cli command>       e.g. "claude"
   cmd:cwd = <working_directory>
   cmd:runonstart = true
   cmd:args = <provider_flags>
   cmd:env = {
       ...env_vars,
       AGENTMUX_AGENT_ID = <agent.name>,
       AGENTMUX_APP_PATH = <agentmux data dir>,
       AGENTMUX_AGENT_CONFIG_ID = <agent.id>
   }
   agent:config_id = <agent.id>
   agent:provider = <provider_id>
   ```
7. Call existing `wcore::create_block(tab_id, meta)` — creates the pane and starts the CLI
8. Register new block with JektRouter under `agent.name`
9. Update `forge_agents.last_launched_at`

### 6.2 Styled Mode Auto-Enable

If the provider supports styled output (Claude, Gemini), automatically set `agentMode: "styled"` and the appropriate `styledArgs` in block meta at launch. The user lands directly in the styled view without having to select it.

### 6.3 Agent Running State

Add `AgentRuntime` in-memory tracking (not persisted — lives only while the server is running):

```rust
pub struct AgentRuntime {
    pub agent_id: String,
    pub block_id: String,
    pub tab_id: String,
    pub status: AgentStatus,  // launching | running | stopping | stopped | crashed
    pub started_at: i64,
    pub last_activity_at: i64,
}
```

Status transitions:
- `launching` → on block create
- `running` → on first PTY output received
- `stopping` → on kill/stop command
- `stopped` → on process exit (clean)
- `crashed` → on unexpected exit (non-zero exit code)

When `crashed` and `restart_on_crash = true`: re-run launch flow, create new block, re-register with JektRouter.

### 6.4 Skills Execution

Skills are defined per-agent in `skills.json`. When a skill trigger matches incoming text (from any source — jekt, user input, scheduled), execute it:

| Skill type | Execution |
|------------|-----------|
| `command` | Inject command into agent's PTY via JektRouter |
| `prompt` | Inject rendered template into agent's PTY |
| `workflow` | Execute steps in sequence (commands/prompts) |
| `mcp` | Expose as MCP tool (future — when MCP server is added) |

Skills execution is triggered by the backend matching incoming jekt messages against trigger patterns.

### 6.5 Verification

```
1. Open Forge → create agent "my-claude" with provider Claude, working_directory = /tmp/test
2. Set AgentMD: "You are a test assistant."
3. Set Soul: "Always respond briefly."
4. Click [Launch]
5. New pane opens in styled mode
6. Verify /tmp/test/CLAUDE.md contains "Always respond briefly.\n\n---\n\nYou are a test assistant."
7. AGENTMUX_AGENT_ID=my-claude is set in the process env
8. Agent registered in JektRouter: curl .../api/jekt/agents shows "my-claude"
```

---

## Phase 7 — Swarm

**Goal:** Make AgentMux swarm-aware. Detect worker tags, group sessions visually, show per-worker status in a SwarmDashboard pane.

### 7.1 Worker Tag Detection

**Format** (from unleashd / oompa convention):
```
[oompa]                            → worker, no group
[oompa:<swarmId>]                  → worker in named swarm
[oompa:<swarmId>:<workerId>]       → fully identified (w0, w1, ...)
```

Tag appears in the **first user message** sent to the session.

**Detection in the Rust backend** (`agentmuxsrv-rs`):

When a jekt or user input is written to a PTY and that PTY has no prior messages, check the message for the oompa tag pattern. If found:
- Set block meta: `swarm:id = <swarmId>`, `swarm:worker_id = <workerId>`, `swarm:role = work`

**Detection in the frontend** (`useAgentStream`):

When the first `user_message` node arrives, regex check for `\[oompa(?::([^:\]]+))?(?::([^:\]]+))?\]`. If matched:
- Dispatch a `setSwarmMeta(blockId, { swarmId, workerId })` action
- Update block meta via RPC

### 7.2 Worker Role Inference

After each completed assistant turn, infer role from content:

```typescript
function inferRole(content: string): "work" | "review" | "fix" {
    if (content.includes("VERDICT")) return "review";
    if (content.startsWith("The reviewer found issues")) return "fix";
    return "work";
}
```

Emit `swarm:role-updated` to update the role badge in the header.

### 7.3 Session Grouping in Tab Bar

Extend the tab bar / session picker to show swarm groups:

```
Sessions
├── [Swarm: refactor-auth] (3 workers)
│   ├── w0 · work · running
│   ├── w1 · review · waiting
│   └── w2 · fix · idle
├── standalone-agent
└── another-agent
```

Group header shows: swarm ID (truncated), worker count, aggregate status (running if any worker running).

Clicking the group header opens the SwarmDashboard pane. Clicking a worker row focuses that pane.

### 7.4 SwarmDashboard Pane

New block view type: `"swarm"`. Created automatically when user clicks a swarm group header, or manually.

```
┌─────────────────────────────────────────────────────────┐
│  Swarm: refactor-auth                      3 workers    │
├─────────────────┬───────────────┬───────────────────────┤
│  w0 · work      │  w1 · review  │  w2 · fix             │
│  ● running      │  ○ waiting    │  ○ idle               │
│  iter: 4        │  iter: 1      │  iter: 0              │
│  Tool: Read     │               │                       │
│  "Analyzing..." │               │                       │
├─────────────────┴───────────────┴───────────────────────┤
│  [Focus w0] [Focus w1] [Focus w2]          [Kill All]   │
└─────────────────────────────────────────────────────────┘
```

Each worker cell shows:
- Worker ID + role badge
- Status (running / waiting / complete / error)
- Iteration count
- Current tool (if running)
- Last output line (truncated to ~40 chars)

Cell is clickable — focuses the worker's full pane.

### 7.5 Swarm Launch from Forge

Add "Launch Swarm" to the Forge UI. User specifies:
- Number of workers (N)
- Coordinator prompt
- Worker task description

On launch:
1. Generate a `swarmId` (UUID short)
2. Launch coordinator agent (no oompa tag, gets system prompt listing workers)
3. Launch N worker agents, each first message seeded with `[oompa:<swarmId>:w0]`, etc.
4. Each worker gets `AGENTMUX_SWARM_ID` and `AGENTMUX_WORKER_ID` env vars
5. Open SwarmDashboard pane showing all workers

### 7.6 Verification

```
1. Open a terminal pane
2. Run a Claude session
3. Send first message: "[oompa:my-swarm:w0] Implement the login feature"
4. Verify: tab bar shows "Swarm: my-swarm" group with w0
5. Role badge updates as session progresses (work → review after VERDICT appears)
6. Open SwarmDashboard → shows worker cell with live status
7. Click worker cell → focuses the agent pane
```

---

## Phase 8 — Agent Output Stream

**Goal:** Expose a subscribable output stream per agent via SSE. Prerequisite for OpenClaw. Reuses the JSONL file watcher built in Phase 1.

### 8.1 SSE Endpoint

**`GET /api/agents/:agent_id/stream`**

- Looks up the agent's `block_id` from JektRouter
- Looks up the agent's `cwd` from `AgentRuntime`
- Resolves the current session JSONL file via `agentwatch::latest_session_file(cwd)`
- Streams new JSONL lines as SSE events:

```
data: {"type":"assistant","message":{"content":[...]}}

data: {"type":"user","message":{"content":"..."}}

```

- Keeps connection open. When a new line is appended to the JSONL, it's immediately forwarded to all SSE subscribers for that agent.

### 8.2 Event Format

Each SSE event is a raw JSONL line. Consumers parse it with the same `jsonlLineToNodes()` mapper as the frontend.

Special heartbeat event every 30s to keep connections alive:

```
: ping

```

### 8.3 Multiple Consumers

Multiple clients can subscribe to the same agent stream simultaneously (OpenClaw, external monitoring tools, etc.). The JSONL watcher only reads the file once — all subscribers share the same watch.

### 8.4 WebSocket Alternative

For consumers that can't use SSE, also expose:

**WebSocket `ws://<host>/api/agents/:agent_id/ws`**

Same data as SSE but over WebSocket. Sends `{type: "line", line: "<jsonl-line>"}` messages.

### 8.5 Verification

```bash
# Start an agent session
# In another terminal, subscribe to its stream:
curl -N http://localhost:<port>/api/agents/my-claude/stream

# Send a message to the agent
curl -X POST .../api/jekt/inject -d '{"target_agent": "my-claude", "message": "list the files"}'

# Verify: SSE stream receives the completed turns as they're written to JSONL
```

---

## Phase 9 — OpenClaw

**Goal:** Wire OpenClaw as a remote access layer. Incoming messages from any OpenClaw channel (WhatsApp, Telegram, Slack, etc.) route to AgentMux agents via JektRouter. Agent responses stream back through the agent output stream to OpenClaw, which delivers them to the originating channel.

### 9.1 Architecture

```
Phone/Desktop (WhatsApp / Telegram / iMessage / Slack / Discord)
    ↓ channel message
OpenClaw Gateway (Node.js, port 18789)
    ↓ POST /api/jekt/inject
AgentMux JektRouter
    ↓ blockcontroller::send_input
PTY → Claude / Gemini / Codex
    ↓ writes to JSONL
AgentMux Agent Output Stream (SSE or WebSocket)
    ↓ completed turn
OpenClaw Gateway
    ↓ channel reply
Phone/Desktop
```

### 9.2 OpenClaw Configuration

OpenClaw config (`~/.openclaw/config.json`) needs an AgentMux target:

```json
{
  "agentmux": {
    "host": "http://localhost:<port>",
    "token": "<agentmux-auth-token>",
    "default_agent": "my-claude",
    "session_map": {
      "+1234567890": "my-claude",
      "telegram:username": "reviewer-agent"
    }
  }
}
```

### 9.3 OpenClaw → AgentMux (Input)

OpenClaw handles incoming message → resolves `target_agent` from `session_map` or `default_agent`:

```typescript
// In OpenClaw's AgentMux channel adapter:
async function handleIncoming(from: string, text: string) {
    const targetAgent = config.session_map[from] ?? config.default_agent;
    await fetch(`${config.host}/api/jekt/inject`, {
        method: "POST",
        headers: {
            "Content-Type": "application/json",
            "Authorization": `Bearer ${config.token}`,
        },
        body: JSON.stringify({
            target_agent: targetAgent,
            message: text,
            source_agent: `openclaw:${from}`,
        }),
    });
}
```

### 9.4 AgentMux → OpenClaw (Output)

OpenClaw subscribes to the agent output stream and converts completed turns to channel replies:

```typescript
async function subscribeToAgent(agentId: string, replyTo: (text: string) => Promise<void>) {
    const resp = await fetch(`${config.host}/api/agents/${agentId}/stream`);
    const reader = resp.body!.getReader();
    const decoder = new TextDecoder();

    let buffer = "";
    while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });

        const lines = buffer.split("\n");
        buffer = lines.pop() ?? "";

        for (const line of lines) {
            if (!line.startsWith("data: ")) continue;
            const jsonl = line.slice(6).trim();
            if (!jsonl) continue;

            try {
                const parsed = JSON.parse(jsonl);
                if (parsed.type === "assistant") {
                    const text = extractTextContent(parsed.message.content);
                    if (text) await replyTo(text);
                }
            } catch {}
        }
    }
}

function extractTextContent(content: ContentBlock[]): string {
    return content
        .filter(b => b.type === "text")
        .map(b => (b as any).text)
        .join("\n");
}
```

### 9.5 Session Mapping

OpenClaw sessions (per sender) map to Forge `AgentRuntime` instances. When a new sender contacts AgentMux for the first time:

1. Check `session_map` config → is there a dedicated agent for this sender?
2. If yes: use that agent
3. If no: use `default_agent` (shared session) or create a new agent instance from a template config

For isolated sessions per sender (privacy-sensitive use cases): add a `session_mode: "isolated"` config option. AgentMux creates a new Forge agent instance per sender, with the sender ID as a tag.

### 9.6 Voice Pipeline

OpenClaw handles transcription (wake-word → audio capture → speech-to-text). The transcribed text comes to AgentMux as a normal `jekt` message. OpenClaw handles TTS (text-to-speech) on the outbound side.

No special handling needed in AgentMux for voice — it's transparent from the agent's perspective.

### 9.7 Cron + Webhook Triggers

OpenClaw's cron and webhook triggers produce messages in the same format as channel messages. They route through the same `/api/jekt/inject` endpoint.

For scheduled tasks: the trigger message can include a prefix that the agent's Soul/AgentMD instructs it to interpret as a scheduled task.

Example cron trigger message:
```
[scheduled-task] Run the daily test suite and report results.
```

### 9.8 Auth

AgentMux needs a simple token-based auth mechanism for OpenClaw to call `/api/jekt/inject` and subscribe to `/api/agents/:id/stream`. Add an `Authorization: Bearer <token>` check on these endpoints. Token configurable in AgentMux settings.

### 9.9 Verification

```
1. Run OpenClaw gateway with AgentMux adapter configured
2. Send a Telegram message: "list the files in my project"
3. Verify: AgentMux jekt routes to "my-claude" agent
4. Claude runs ls/Read tool
5. Completed turn arrives in SSE stream
6. OpenClaw sends reply back to Telegram
7. Message received on phone
```

---

## Summary: Dependency Graph

```
Phase 1 (Styled view)      ← can start immediately
Phase 2 (Wire jekt)        ← can start immediately, parallel to Phase 1
    ↓
Phase 3 (Forge storage)    ← after Phase 2
    ↓
Phase 4 (Forge UI)         ← after Phase 3
    ↓
Phase 5 (JektRouter)       ← after Phase 2
    ↓
Phase 6 (Forge launch)     ← after Phases 4 + 5
    ↓
Phase 7 (Swarm)            ← after Phase 1 (for styled view) + Phase 5 (for JektRouter)
    ↓
Phase 8 (Output stream)    ← after Phase 5 (needs JektRouter for agent lookup)
    ↓
Phase 9 (OpenClaw)         ← after Phases 5 + 8
```

Phases 1 and 2 are independent and can be built in parallel. Phase 7 (Swarm) can begin as soon as Phase 1 lands — the minimal tag-detection + grouping slice doesn't need JektRouter.

---

## File Index

### New Rust files

| File | Phase |
|------|-------|
| `agentmuxsrv-rs/src/backend/agentwatch/mod.rs` | 1 |
| `agentmuxsrv-rs/src/backend/agentwatch/watcher.rs` | 1 |
| `agentmuxsrv-rs/src/backend/forge/mod.rs` | 3 |
| `agentmuxsrv-rs/src/backend/forge/types.rs` | 3 |
| `agentmuxsrv-rs/src/backend/forge/storage.rs` | 3 |
| `agentmuxsrv-rs/src/backend/forge/watcher.rs` | 3 |
| `agentmuxsrv-rs/src/backend/forge/launch.rs` | 6 |
| `agentmuxsrv-rs/src/server/forge.rs` | 3 |
| `agentmuxsrv-rs/src/backend/jekt/mod.rs` | 5 |
| `agentmuxsrv-rs/src/backend/jekt/router.rs` | 5 |
| `agentmuxsrv-rs/src/backend/jekt/types.rs` | 5 |
| `agentmuxsrv-rs/src/backend/jekt/lan.rs` | 5 |
| `agentmuxsrv-rs/src/backend/jekt/cloud.rs` | 5 |
| `agentmuxsrv-rs/src/server/jekt.rs` | 5 |

### Modified Rust files

| File | Phase | Change |
|------|-------|--------|
| `agentmuxsrv-rs/src/main.rs` | 2 | Wire jekt input sender |
| `agentmuxsrv-rs/src/main.rs` | 5 | Create JektRouter, add to AppState |
| `agentmuxsrv-rs/src/backend/mod.rs` | 1, 3, 5 | Add agentwatch, forge, jekt modules |
| `agentmuxsrv-rs/src/backend/storage/wstore.rs` | 3 | Run forge migrations |
| `agentmuxsrv-rs/src/server/mod.rs` | 1, 3, 5, 8 | Add routes |

### New Frontend files

| File | Phase |
|------|-------|
| `frontend/app/view/agent/jsonl-types.ts` | 1 |
| `frontend/app/view/agent/jsonl-mapper.ts` | 1 |
| `frontend/app/view/agent/useAgentStream.ts` | 1 |
| `frontend/app/view/agent/components/AgentDocumentView.tsx` | 1 |
| `frontend/app/view/agent/components/TerminalOutputBlock.tsx` | 1 |
| `frontend/app/view/forge/forge-view.tsx` | 4 |
| `frontend/app/view/forge/agent-list.tsx` | 4 |
| `frontend/app/view/forge/agent-detail.tsx` | 4 |
| `frontend/app/view/forge/content-preview.tsx` | 4 |
| `frontend/app/view/forge/skills-panel.tsx` | 4 |
| `frontend/app/view/forge/forge-model.ts` | 4 |
| `frontend/app/view/forge/api.ts` | 4 |
| `frontend/app/view/forge/types.ts` | 4 |
| `frontend/app/view/forge/forge.scss` | 4 |
| `frontend/app/view/forge/index.ts` | 4 |
| `frontend/app/view/swarm/swarm-dashboard.tsx` | 7 |
| `frontend/app/view/swarm/swarm-model.ts` | 7 |
| `frontend/app/view/swarm/swarm.scss` | 7 |

### Modified Frontend files

| File | Phase | Change |
|------|-------|--------|
| `frontend/app/view/agent/types.ts` | 1 | Add TerminalOutputNode to DocumentNode union |
| `frontend/app/view/agent/state.ts` | 1 | Add appendNodeAtom, updateNodeAtom |
| `frontend/app/view/agent/agent-view.tsx` | 1 | Replace AgentStyledSession placeholder |
| `frontend/app/view/agent/agent-view.scss` | 1 | Add layout + terminal output styles |
