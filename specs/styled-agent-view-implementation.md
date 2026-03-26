# Styled Agent View — Implementation Spec

**Author:** AgentX
**Date:** 2026-03-08
**Status:** Draft
**Base Version:** 0.31.80
**Priority:** Critical — this is the core product differentiator

---

## Table of Contents

1. [What Exists vs What's Missing](#1-what-exists-vs-whats-missing)
2. [Data Flow Architecture](#2-data-flow-architecture)
3. [Step 1 — Stream Subscription Hook](#3-step-1--stream-subscription-hook)
4. [Step 2 — Document Renderer Component](#4-step-2--document-renderer-component)
5. [Step 3 — Message Input Wiring](#5-step-3--message-input-wiring)
6. [Step 4 — Styled Session Launch Flow](#6-step-4--styled-session-launch-flow)
7. [Step 5 — Polish & Edge Cases](#7-step-5--polish--edge-cases)
8. [File Map](#8-file-map)
9. [Testing](#9-testing)

---

## 1. What Exists vs What's Missing

### Fully Built (Ready to Use)

| Component | File | Lines | Notes |
|-----------|------|-------|-------|
| `ClaudeCodeStreamParser` | `stream-parser.ts` | 309 | NDJSON → DocumentNode, tool summaries, collapse logic |
| `ClaudeTranslator` | `providers/claude-translator.ts` | 286 | Anthropic stream-json → internal StreamEvent format |
| Provider definitions | `providers/index.ts` | 81 | CLI commands, styledArgs (`--output-format stream-json --verbose`), auth flows |
| Translator factory | `providers/translator-factory.ts` | 25 | Route output format → translator |
| State atoms | `state.ts` | 345 | Per-instance Jotai atoms: document, process, streaming, auth, filters |
| Type system | `types.ts` | 310 | DocumentNode union (markdown, tool, agent_message, user_message), StreamEvent |
| `AgentHeader` | `components/AgentHeader.tsx` | 83 | Status bar, PID, connection state |
| `AgentFooter` | `components/AgentFooter.tsx` | 54 | Textarea + Shift+Enter send |
| `ToolBlock` | `components/ToolBlock.tsx` | 131 | Collapsible tool display, per-tool rendering |
| `MarkdownBlock` | `components/MarkdownBlock.tsx` | 30 | Rendered markdown with thinking support |
| `AgentMessageBlock` | `components/AgentMessageBlock.tsx` | 53 | Inter-agent mux/ject messages |
| `DiffViewer` | `components/DiffViewer.tsx` | 51 | Unified diff with line coloring |
| `BashOutputViewer` | `components/BashOutputViewer.tsx` | 49 | Command + stdout/stderr + exit code |
| `ConnectionStatus` | `components/ConnectionStatus.tsx` | 199 | OAuth/API key auth UI |
| `ProcessControls` | `components/ProcessControls.tsx` | 97 | Pause/resume/kill/restart |
| `FilterControls` | `components/FilterControls.tsx` | 82 | Thinking/tools/messages visibility |
| `InitializationMonitor` | `init-monitor.ts` | 266 | Detects theme/login prompts from PTY |
| `InitializationPrompt` | `components/InitializationPrompt.tsx` | exists | UI for init question responses |
| `SetupWizard` | `components/SetupWizard.tsx` | exists | Multi-step CLI setup |
| Agent ViewModel | `agent-model.ts` | 147 | Provider config, styled/raw mode switch |
| Provider picker UI | `agent-view.tsx` | 187 | 3 provider buttons, mode selection |

### Missing (Must Build)

| Piece | Estimated Lines | What It Does |
|-------|----------------|--------------|
| **Stream subscription hook** | ~120 | Subscribes to block PTY file subject, pipes through translator + parser, updates atoms |
| **Document renderer component** | ~180 | Renders DocumentNode[] as scrollable styled output |
| **Message input wiring** | ~30 | AgentFooter → RpcApi.ControllerInputCommand → PTY |
| **Styled session view assembly** | ~80 | Replaces the "Starting session..." placeholder with Header + Document + Footer |

**Total: ~410 lines across 3-4 files.**

---

## 2. Data Flow Architecture

### Current Terminal Flow (Working)

```
PTY (backend)
  → FileUpdate event via WebSocket
  → getFileSubject(blockId, "term") → RxJS Subject
  → TermWrap.handleNewFileSubjectData()
  → base64 decode → terminal.write() (xterm.js raw bytes)
```

### Styled Agent Flow (To Build)

```
PTY (backend) — Claude runs with --output-format stream-json --verbose
  → FileUpdate event via WebSocket
  → getFileSubject(blockId, "term") → RxJS Subject
  → useAgentStream() hook
      → base64 decode → UTF-8 text
      → split into NDJSON lines
      → ClaudeTranslator.translate(line) → StreamEvent
      → ClaudeCodeStreamParser.parseLine(line) → DocumentNode
      → appendNodeAtom → documentAtom updated
  → AgentDocumentView reads documentAtom
      → renders DocumentNode[] as MarkdownBlock / ToolBlock / etc.
```

### User Input Flow (To Build)

```
AgentFooter textarea
  → onSendMessage(text)
  → RpcApi.ControllerInputCommand(blockId, base64(text + "\n"))
  → Backend writes to PTY stdin
  → Claude receives user message
```

### Key Functions Referenced

From `termwrap.ts` — the existing subscription pattern to replicate:

```typescript
// Subscribe to PTY output (line 595-596)
this.mainFileSubject = getFileSubject(this.blockId, TermFileName);
this.mainFileSubject.subscribe(this.handleNewFileSubjectData.bind(this));

// Send input to PTY (from termViewModel.ts line 380-383)
sendDataToController(data: string) {
    const b64data = stringToBase64(data);
    RpcApi.ControllerInputCommand(TabRpcClient, {
        blockid: this.blockId,
        inputdata64: b64data,
    });
}
```

From `termwrap.ts` — file subject data format:

```typescript
// WSFileEventData has:
// fileop: "append" | "truncate"
// data64: string (base64 encoded bytes)
handleNewFileSubjectData(msg: WSFileEventData) {
    if (msg.fileop == "truncate") { /* clear */ }
    else if (msg.fileop == "append") {
        const decodedData = base64ToArray(msg.data64);
        // For styled view: decode as UTF-8 text, parse as NDJSON
    }
}
```

---

## 3. Step 1 — Stream Subscription Hook

**File:** `frontend/app/view/agent/useAgentStream.ts` `[NEW]`

This hook subscribes to the block's PTY file subject and pipes data through the translator + parser pipeline, updating Jotai atoms.

```typescript
// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useAgentStream — Subscribes to PTY NDJSON output and converts to DocumentNodes.
 *
 * Data flow:
 * PTY FileSubject → base64 decode → UTF-8 text → NDJSON lines
 *   → Translator (provider-specific) → StreamEvent
 *   → StreamParser → DocumentNode
 *   → Jotai atom update
 */

import { useEffect, useRef } from "react";
import { useAtom, useSetAtom } from "jotai";
import { getFileSubject, base64ToArray } from "@/app/store/wps";
import { globalStore } from "@/app/store/global";
import { createTranslator } from "./providers/translator-factory";
import { ClaudeCodeStreamParser } from "./stream-parser";
import type { AgentAtoms } from "./state";
import type { DocumentNode } from "./types";
import type { ProviderDefinition } from "./providers";

const TermFileName = "term";

interface UseAgentStreamOpts {
    blockId: string;
    provider: ProviderDefinition;
    atoms: AgentAtoms;
    enabled: boolean;
}

export function useAgentStream({ blockId, provider, atoms, enabled }: UseAgentStreamOpts) {
    const appendNode = useSetAtom(atoms.appendNodeAtom);
    const updateNode = useSetAtom(atoms.updateNodeAtom);
    const setStreaming = useSetAtom(atoms.streamingStateAtom);

    // Persistent refs across renders
    const parserRef = useRef<ClaudeCodeStreamParser | null>(null);
    const bufferRef = useRef<string>("");
    const decoderRef = useRef(new TextDecoder());

    useEffect(() => {
        if (!enabled || !blockId) return;

        // Initialize parser and translator
        const parser = new ClaudeCodeStreamParser();
        parserRef.current = parser;
        bufferRef.current = "";

        const translator = createTranslator(provider.styledOutputFormat);

        // Subscribe to PTY file subject (same pattern as TermWrap)
        const fileSubject = getFileSubject(blockId, TermFileName);

        const subscription = fileSubject.subscribe((msg) => {
            if (msg.fileop === "truncate") {
                // Clear document
                parser.reset();
                bufferRef.current = "";
                globalStore.set(atoms.documentAtom, []);
                return;
            }

            if (msg.fileop !== "append") return;

            // Decode base64 → bytes → UTF-8 text
            const bytes = base64ToArray(msg.data64);
            const text = decoderRef.current.decode(bytes, { stream: true });

            // Buffer and split into complete NDJSON lines
            bufferRef.current += text;
            const lines = bufferRef.current.split("\n");
            bufferRef.current = lines.pop() || ""; // Keep incomplete line

            for (const line of lines) {
                if (!line.trim()) continue;

                try {
                    // Step 1: Translate provider-specific format → StreamEvent
                    const events = translator.translate(line);

                    for (const event of events) {
                        // Step 2: Parse StreamEvent → DocumentNode
                        const node = parser.parseLine(JSON.stringify(event));
                        if (!node) continue;

                        // Step 3: Update atom
                        // Tool results replace existing tool_call nodes (same ID)
                        if (node.type === "tool" && node.status !== "running") {
                            // This is a tool_result — update the existing node
                            globalStore.set(atoms.updateNodeAtom, node);
                        } else {
                            // New node — append
                            globalStore.set(atoms.appendNodeAtom, node);
                        }
                    }
                } catch (err) {
                    // Not valid JSON — might be raw PTY output (ANSI, prompts, etc.)
                    // For styled mode, we can either ignore or show as raw text
                    console.debug("[agent-stream] non-JSON line:", line.slice(0, 100));
                }
            }
        });

        setStreaming(true);

        return () => {
            subscription.unsubscribe?.();
            fileSubject.release();
            setStreaming(false);
            parserRef.current = null;
        };
    }, [blockId, provider.id, enabled]);
}
```

### State Atom Updates Needed

Check `state.ts` for `appendNodeAtom` and `updateNodeAtom`. If they don't exist with the right signatures, they need to be added:

```typescript
// In state.ts, inside createAgentAtoms():

// Append a new node to the document
const appendNodeAtom = atom(null, (get, set, node: DocumentNode) => {
    const doc = get(documentAtom);
    set(documentAtom, [...doc, node]);
});

// Update an existing node by ID (for tool_result replacing tool_call)
const updateNodeAtom = atom(null, (get, set, node: DocumentNode) => {
    const doc = get(documentAtom);
    const idx = doc.findIndex(n => n.id === node.id);
    if (idx >= 0) {
        const updated = [...doc];
        updated[idx] = node;
        set(documentAtom, updated);
    } else {
        // ID not found — append instead
        set(documentAtom, [...doc, node]);
    }
});
```

---

## 4. Step 2 — Document Renderer Component

**File:** `frontend/app/view/agent/components/AgentDocumentView.tsx` `[NEW]`

Renders the document atom as a scrollable list of styled blocks.

```tsx
// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentDocumentView — Renders DocumentNode[] as styled output.
 *
 * Maps node types to existing components:
 * - "markdown" → MarkdownBlock
 * - "tool" → ToolBlock
 * - "agent_message" → AgentMessageBlock
 * - "user_message" → UserMessageBlock (inline)
 */

import React, { memo, useRef, useEffect } from "react";
import { useAtomValue } from "jotai";
import { MarkdownBlock } from "./MarkdownBlock";
import { ToolBlock } from "./ToolBlock";
import { AgentMessageBlock } from "./AgentMessageBlock";
import type { AgentAtoms } from "../state";
import type { DocumentNode, MarkdownNode, ToolNode, AgentMessageNode, UserMessageNode } from "../types";

interface Props {
    atoms: AgentAtoms;
}

export const AgentDocumentView = memo(({ atoms }: Props) => {
    const document = useAtomValue(atoms.documentAtom);
    const filters = useAtomValue(atoms.filterAtom);
    const scrollRef = useRef<HTMLDivElement>(null);

    // Auto-scroll to bottom when new nodes arrive
    useEffect(() => {
        if (scrollRef.current) {
            const el = scrollRef.current;
            const isNearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 100;
            if (isNearBottom) {
                el.scrollTop = el.scrollHeight;
            }
        }
    }, [document.length]);

    // Apply filters
    const filtered = document.filter((node) => {
        if (node.type === "markdown" && node.metadata?.thinking && !filters.showThinking) {
            return false;
        }
        if (node.type === "tool" && !filters.showTools) {
            return false;
        }
        return true;
    });

    return (
        <div className="agent-document-view" ref={scrollRef}>
            {filtered.length === 0 && (
                <div className="agent-document-empty">
                    Waiting for output...
                </div>
            )}
            {filtered.map((node) => (
                <DocumentNodeRenderer key={node.id} node={node} atoms={atoms} />
            ))}
        </div>
    );
});

const DocumentNodeRenderer = memo(({ node, atoms }: { node: DocumentNode; atoms: AgentAtoms }) => {
    switch (node.type) {
        case "markdown":
            return <MarkdownBlock node={node as MarkdownNode} />;

        case "tool":
            return <ToolBlock node={node as ToolNode} atoms={atoms} />;

        case "agent_message":
            return <AgentMessageBlock node={node as AgentMessageNode} />;

        case "user_message":
            return <UserMessageBlock node={node as UserMessageNode} />;

        default:
            return null;
    }
});

/** Inline user message display */
const UserMessageBlock = memo(({ node }: { node: UserMessageNode }) => {
    return (
        <div className="agent-user-message">
            <div className="agent-user-message-header">
                <span className="agent-user-icon">👤</span>
                <span className="agent-user-label">You</span>
            </div>
            <div className="agent-user-message-body">
                {node.message}
            </div>
        </div>
    );
});
```

---

## 5. Step 3 — Message Input Wiring

**File:** `frontend/app/view/agent/agent-view.tsx` `[MOD]`

Wire `AgentFooter.onSendMessage` to write to the PTY via `RpcApi.ControllerInputCommand`.

The existing `AgentFooter` component exposes an `onSendMessage` prop. We need to:

```typescript
import { RpcApi, TabRpcClient } from "@/app/store/wshclientapi";
import { stringToBase64 } from "@/util/util";

// Inside the styled session component:
const handleSendMessage = useCallback((text: string) => {
    if (!text.trim()) return;

    // Send to PTY as if user typed it + Enter
    const b64data = stringToBase64(text + "\n");
    RpcApi.ControllerInputCommand(TabRpcClient, {
        blockid: blockId,
        inputdata64: b64data,
    });

    // Optionally add a local user_message node for immediate display
    // (Claude's stream-json output will also echo user messages)
    globalStore.set(atoms.appendNodeAtom, {
        type: "user_message",
        id: `user_${Date.now()}`,
        message: text,
        timestamp: Date.now(),
        collapsed: false,
        summary: "👤 User Message",
    });
}, [blockId]);
```

---

## 6. Step 4 — Styled Session Launch Flow

### Current Flow (agent-view.tsx)

When user clicks a provider button with "Styled" mode:

1. `connectStyled(providerId)` is called
2. Sets block meta: `agentMode: "styled"`, `agentProvider: providerId`
3. Sets CLI args including `--output-format stream-json --verbose`
4. Calls `ControllerResyncCommand` to start the CLI

The styled session component currently shows a placeholder spinner.

### New Flow

Replace the placeholder in `AgentStyledSession` (in `agent-view.tsx`) with the full styled view:

**`frontend/app/view/agent/agent-view.tsx`** `[MOD]`

Replace the existing `AgentStyledSession` component body:

```tsx
import { useAgentStream } from "./useAgentStream";
import { AgentDocumentView } from "./components/AgentDocumentView";
import { AgentHeader } from "./components/AgentHeader";
import { AgentFooter } from "./components/AgentFooter";
import { FilterControls } from "./components/FilterControls";
import { getProvider } from "./providers";
import { createAgentAtoms } from "./state";

function AgentStyledSession({ blockId, model }: { blockId: string; model: AgentViewModel }) {
    const [blockData] = useWaveObjectValue<Block>(makeORef("block", blockId));
    const providerId = blockData?.meta?.["agentProvider"] ?? "claude";
    const provider = getProvider(providerId);

    // Get or create per-block atoms
    const atoms = useMemo(() => createAgentAtoms(blockId), [blockId]);

    // Subscribe to PTY stream and parse into document nodes
    useAgentStream({
        blockId,
        provider,
        atoms,
        enabled: true,
    });

    // Send message handler
    const handleSendMessage = useCallback((text: string) => {
        if (!text.trim()) return;
        const b64data = stringToBase64(text + "\n");
        RpcApi.ControllerInputCommand(TabRpcClient, {
            blockid: blockId,
            inputdata64: b64data,
        });
    }, [blockId]);

    // Disconnect handler — return to provider picker
    const handleDisconnect = useCallback(() => {
        model.disconnectStyled();
    }, [model]);

    return (
        <div className="agent-styled-session">
            <AgentHeader
                blockId={blockId}
                provider={provider}
                atoms={atoms}
                onDisconnect={handleDisconnect}
            />
            <FilterControls atoms={atoms} />
            <AgentDocumentView atoms={atoms} />
            <AgentFooter onSendMessage={handleSendMessage} />
        </div>
    );
}
```

### CSS Layout

**`frontend/app/view/agent/agent.scss`** `[MOD]` (or new file)

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
    scroll-behavior: smooth;
}

.agent-document-empty {
    color: var(--text-secondary);
    text-align: center;
    padding: 40px;
    font-style: italic;
}

.agent-user-message {
    margin: 8px 0;
    padding: 8px 12px;
    border-left: 3px solid var(--accent-color);
    background: var(--bg-secondary);
    border-radius: 4px;
}

.agent-user-message-header {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--text-secondary);
    margin-bottom: 4px;
}

.agent-user-message-body {
    white-space: pre-wrap;
    font-family: var(--mono-font);
    font-size: 13px;
}
```

---

## 7. Step 5 — Polish & Edge Cases

### 7.1 Handle Raw PTY Output Mixed with JSON

Claude Code with `--output-format stream-json` still emits some non-JSON output (ANSI escape sequences, progress bars, initialization prompts). The stream hook should gracefully skip non-JSON lines.

Already handled in Step 1 with the try/catch around `translator.translate(line)`.

For init prompts, the existing `InitializationMonitor` should still receive raw PTY chunks. Wire it in `useAgentStream`:

```typescript
// In useAgentStream, before the NDJSON parsing:
if (initMonitor.current?.active) {
    initMonitor.current.handleRawOutput(text);
}
```

### 7.2 Tool Node Updates

When a `tool_result` event arrives, it should **replace** the existing `tool_call` node with the same ID. This is handled by `updateNodeAtom` in Step 1.

The ToolBlock component should re-render with:
- Status changing from "running" (spinner) to "success" (checkmark) or "error" (X)
- Collapsed state: successes auto-collapse, errors stay expanded
- Result content displayed when expanded

### 7.3 Auto-Scroll Behavior

From Step 2: only auto-scroll if the user is already near the bottom (within 100px). If they've scrolled up to read earlier output, don't jump them back down.

### 7.4 Large Documents

For long sessions with 1000+ nodes:
- Use `react-window` or similar virtualization if performance becomes an issue
- Keep collapsed nodes lightweight (just the summary line)
- Consider pruning very old nodes (configurable limit)

### 7.5 Disconnect / Return to Provider Picker

The `agent-model.ts` already has `disconnectStyled()` which resets block meta. When triggered:
1. Stream subscription unsubscribes (cleanup in useEffect)
2. Document atoms cleared
3. View returns to provider picker

### 7.6 Raw Mode Toggle

Users should be able to toggle between styled view and raw terminal mid-session. This is already partially supported:
- The terminal PTY runs regardless of view mode
- Switching to raw mode just renders xterm.js on the same PTY
- Switching back to styled mode re-subscribes to the file subject

The stream parser should be able to catch up by replaying the PTY file from the beginning (using `loadInitialTerminalData` pattern from termwrap.ts).

### 7.7 Translator Output Format

The `ClaudeTranslator.translate(line)` method expects the raw NDJSON line as input and returns `StreamEvent[]`. Check if its interface matches:

```typescript
// From translator.ts:
export interface OutputTranslator {
    translate(rawLine: string): StreamEvent[];
}
```

If the translator expects the already-parsed JSON (not the raw string), adjust the pipeline:
```typescript
const parsed = JSON.parse(line);
const events = translator.translate(parsed);
// Then skip the JSON.stringify in parser.parseLine
```

Verify by reading the exact `translate()` signature in `claude-translator.ts`.

---

## 8. File Map

### New Files

| File | Lines | Purpose |
|------|-------|---------|
| `fe/app/view/agent/useAgentStream.ts` | ~120 | PTY subscription → translator → parser → atoms |
| `fe/app/view/agent/components/AgentDocumentView.tsx` | ~100 | Renders DocumentNode[] with type routing |

### Modified Files

| File | Change |
|------|--------|
| `fe/app/view/agent/agent-view.tsx` | Replace `AgentStyledSession` placeholder with full styled view |
| `fe/app/view/agent/state.ts` | Add `appendNodeAtom` and `updateNodeAtom` if missing |
| `fe/app/view/agent/agent.scss` (or new) | Layout styles for styled session |

### No Changes Needed

All existing components (`ToolBlock`, `MarkdownBlock`, `AgentMessageBlock`, `AgentHeader`, `AgentFooter`, `FilterControls`, `ProcessControls`, `ConnectionStatus`, `DiffViewer`, `BashOutputViewer`) are ready to use as-is.

---

## 9. Testing

### Manual Testing Checklist

1. **Launch styled session:**
   - Open agent pane → click Claude → select "Styled" mode
   - Verify CLI starts with `--output-format stream-json --verbose`
   - Verify NDJSON output is parsed and displayed as styled blocks

2. **Document rendering:**
   - Text output → renders as MarkdownBlock with formatted markdown
   - Thinking blocks → appear with thinking indicator, filterable
   - Tool calls → appear with spinner (running), then update to success/error
   - Tool results → replace the running tool node, success collapses, error expands

3. **User input:**
   - Type in footer → press Enter → message appears in document view
   - Claude receives the message and responds
   - New response nodes appear in the document

4. **Scrolling:**
   - Output auto-scrolls to bottom
   - Scroll up to read earlier output → new output does NOT jump scroll
   - Scroll back to bottom → auto-scroll resumes

5. **Filters:**
   - Toggle "Thinking" off → thinking blocks hidden
   - Toggle "Tools" off → tool blocks hidden
   - Counts update in filter controls

6. **Disconnect:**
   - Click disconnect → returns to provider picker
   - Re-connect → new session, clean document

7. **Process controls:**
   - Kill agent → process stops, status updates
   - Restart → new session begins

### Automated Tests

```typescript
// useAgentStream.test.ts
describe("useAgentStream", () => {
    it("parses NDJSON lines into document nodes", () => {
        // Mock file subject with sample NDJSON
        // Verify atoms are updated with correct node types
    });

    it("handles incomplete lines across chunks", () => {
        // Send a line split across two chunks
        // Verify it's reassembled correctly
    });

    it("skips non-JSON lines gracefully", () => {
        // Send ANSI escape sequences, plain text
        // Verify no errors, no nodes created
    });

    it("updates tool nodes on tool_result", () => {
        // Send tool_call then tool_result with same ID
        // Verify node is updated, not duplicated
    });
});

// AgentDocumentView.test.tsx
describe("AgentDocumentView", () => {
    it("renders markdown nodes as MarkdownBlock", () => { });
    it("renders tool nodes as ToolBlock", () => { });
    it("filters thinking blocks when disabled", () => { });
    it("auto-scrolls to bottom on new nodes", () => { });
});
```

---

## Appendix: Translator Pipeline Detail

### Claude Code `--output-format stream-json` Output

Each line is a JSON object:
```json
{"type":"stream_event","event":{"type":"message_start","message":{"id":"msg_01...","type":"message","role":"assistant"}}}
{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}}
{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Let me "}}}
{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"check that."}}}
{"type":"stream_event","event":{"type":"content_block_stop","index":0}}
{"type":"stream_event","event":{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_01...","name":"Read","input":{}}}}
{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"file_path\""}}}
{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":":\"/src/main.ts\"}"}}}
{"type":"stream_event","event":{"type":"content_block_stop","index":1}}
```

### ClaudeTranslator Pipeline

1. Parse outer `{"type":"stream_event","event":{...}}` wrapper
2. Extract inner Anthropic event
3. Accumulate text deltas into complete text blocks
4. Accumulate `input_json_delta` into complete tool call params
5. Emit `StreamEvent` objects: `{type: "text", content: "Let me check that."}`

### ClaudeCodeStreamParser Pipeline

1. Receives `StreamEvent` (already translated)
2. Converts to `DocumentNode`:
   - `text` → `{type: "markdown", content: "..."}`
   - `tool_call` → `{type: "tool", tool: "Read", params: {...}, status: "running"}`
   - `tool_result` → `{type: "tool", tool: "Read", status: "success", result: "..."}`
3. Generates summary strings with icons

### End-to-End

```
Raw NDJSON line
  → ClaudeTranslator.translate() → StreamEvent[]
  → ClaudeCodeStreamParser.parseLine() → DocumentNode
  → Jotai atom → React re-render → Styled UI
```
