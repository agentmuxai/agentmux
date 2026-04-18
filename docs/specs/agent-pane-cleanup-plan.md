# Agent Pane Architecture & Cleanup Plan

**Date:** 2026-04-15  
**Author:** AgentA  
**Status:** Proposed

---

## 1. Current State

### File Inventory

| File | Lines | Role |
|------|-------|------|
| `agent-view.tsx` | 352 | Root component — wires everything together |
| `agent-model.ts` | 505 | ViewModel: atoms, launch, input dispatch |
| `state.ts` | ~120 | `createAgentAtoms()` — all signal pairs |
| `types.ts` | 426 | All shared TypeScript types |
| `useAgentStream.ts` | 376 | WPS output subscription + stream parsing |
| `stream-parser.ts` | 357 | `ClaudeCodeStreamParser` — events → DocumentNodes |
| `init-monitor.ts` | 262 | Auth/onboarding detection from output bytes |
| `buildRuntimeArgs.ts` | ~80 | CLI flag builder |
| `parseHistoryLines.ts` | ~60 | NDJSON → DocumentNodes for history load |
| `index.ts` | ~20 | Re-exports |
| **components/** | | |
| `AgentDocumentView.tsx` | 513 | Document scroll container + node renderer |
| `ToolBlock.tsx` | 376 | Tool call/result block renderer |
| `AgentControlBar.tsx` | 339 | Top control bar (provider, model, etc.) |
| `AgentFooter.tsx` | 332 | Input box + status |
| `AgentPicker.tsx` | 286 | Agent selector |
| `DiffViewer.tsx` | 209 | Diff renderer for Write tool |
| `AgentCard.tsx` | 209 | Individual agent card |
| `AgentIdentityPanel.tsx` | 189 | Agent identity UI |
| `AgentCardSettingsPanel.tsx` | 173 | Agent settings panel |
| `SlashCommandPicker.tsx` | 129 | Slash command dropdown |
| `SlashHelpPanel.tsx` | 135 | Slash command help overlay |
| `CompactResult.tsx` | 121 | Collapsed result display |
| `BookmarksPanel.tsx` | 142 | Bookmarks UI |
| `BashOutputViewer.tsx` | ~80 | Bash output block |
| `MarkdownBlock.tsx` | ~70 | Markdown renderer |
| `HighlightedCode.tsx` | 139 | Code block with syntax highlighting |
| `SessionDigestBanner.tsx` | ~60 | Session summary banner |
| `SubagentLinkBlock.tsx` | ~50 | Subagent reference block |
| `NewAgentCard.tsx` | ~40 | "New agent" card |
| `FilterControls.tsx` | ~60 | Document filter controls |
| `AgentSearchBar.tsx` | ~80 | In-session search bar |
| `SlashAutocomplete.tsx` | ~60 | Slash autocomplete overlay |
| **DEAD components** | | |
| `SetupWizard.tsx` | 437 | **Orphan — mounted nowhere** |
| `ConnectionStatus.tsx` | 192 | **Orphan — mounted nowhere** |
| `AgentHeader.tsx` | ~80 | **Orphan — mounted nowhere** |
| `ProcessControls.tsx` | ~60 | **Orphan — mounted nowhere** |
| `InitializationPrompt.tsx` | 117 | **Orphan — mounted nowhere** |
| **hooks/** | | |
| `useAgentCommands.ts` | 281 | Command dispatch + slash handling |
| `useAgentControllerStatus.ts` | 158 | WPS controller status subscription |
| `useHistoryPagination.ts` | 182 | Paginated history load |
| `useBookmarks.ts` | 158 | Bookmark CRUD |
| `useInSessionSearch.ts` | 142 | Document text search |
| `useScrollToNode.ts` | ~80 | Auto-scroll to node |
| `useAgentKeyboard.ts` | ~60 | Keyboard shortcut handling |
| `useSessionDigest.ts` | 115 | Session digest generation |
| `useSubagentEvents.ts` | ~80 | Subagent spawn/status events |
| `useControllerStatusEvents.ts` | ~60 | Translates WPS → log entries |
| `useLaunchLogs.ts` | ~60 | Launch log buffer |
| **flows/** | | |
| `launch-flow.ts` | 272 | 3-phase startup (runtime → CLI → auth → resync) |
| **commands/** | | |
| `dispatch.ts` | 171 | Command router |
| `registry.ts` | 116 | Slash command registry |
| `parse.ts` | ~60 | Slash command parser |
| `types.ts` | 160 | Command type contracts |
| `global/{clear,help,login,runtime,tools}.ts` | ~40 each | Built-in slash commands |
| `providers/{claude,codex,index}.ts` | ~40 each | Provider-specific commands |
| **providers/** | | |
| `claude-translator.ts` | 334 | Claude stream-json → StreamEvent |
| `codex-translator.ts` | 147 | OpenAI/Codex format translator |
| `gemini-translator.ts` | ~80 | Gemini format translator |
| `translator-factory.ts` | ~30 | `createTranslator(format)` factory |
| `translator.ts` | ~60 | Base `ITranslator` interface |
| `index.ts` + `index.test.ts` | ~120+118 | Exports + unit tests |

**Total: ~70 files, ~10,800 lines**

---

## 2. Dependency Graph (simplified)

```
agent-view.tsx
├── agent-model.ts           ← all atoms + launch logic
│   ├── state.ts             ← createAgentAtoms()
│   ├── flows/launch-flow.ts
│   ├── init-monitor.ts
│   └── buildRuntimeArgs.ts
├── useAgentStream.ts        ← WPS output → DocumentNodes
│   ├── providers/translator-factory.ts
│   └── stream-parser.ts
├── components/AgentDocumentView.tsx
│   ├── components/ToolBlock.tsx
│   ├── components/MarkdownBlock.tsx
│   ├── components/DiffViewer.tsx
│   └── ... (all renderers)
├── components/AgentControlBar.tsx
├── components/AgentFooter.tsx
│   ├── hooks/useAgentCommands.ts
│   └── components/SlashCommandPicker.tsx
├── hooks/useAgentControllerStatus.ts
├── hooks/useHistoryPagination.ts
│   └── parseHistoryLines.ts
└── hooks/ (all others)
```

---

## 3. State Ownership Map

### Active atoms (read + written from live code)

| Atom | Written by | Read by |
|------|-----------|---------|
| `documentAtom` | `useAgentStream`, `useHistoryPagination` | `AgentDocumentView`, `useInSessionSearch`, `useBookmarks` |
| `documentStateAtom` | `useHistoryPagination` | `AgentFooter`, `AgentDocumentView` |
| `streamingStateAtom` | `useAgentStream` | `AgentFooter`, status bar |
| `sessionStatsAtom` | `useAgentStream` | `AgentFooter` |
| `currentToolAtom` | `useAgentStream` | status bar |
| `turnTokensAtom` | `useAgentStream` | status bar |
| `turnActiveAtom` | `useAgentStream`, `agent-model` | `AgentFooter`, input guard |

### Dead atoms — created in `state.ts`, never written from live code

| Atom | Created | Consumers | Status |
|------|---------|-----------|--------|
| `processAtom` | `state.ts` | `AgentHeader.tsx` (orphan), `ProcessControls.tsx` (orphan) | **DEAD** |
| `messageRouterAtom` | `state.ts` | `AgentHeader.tsx` (orphan) | **DEAD** |
| `authAtom` | `state.ts` | `ConnectionStatus.tsx` (orphan) | **DEAD** |
| `userInfoAtom` | `state.ts` | `ConnectionStatus.tsx` (orphan) | **DEAD** |
| `providerConfigAtom` | `state.ts` | `ConnectionStatus.tsx` (orphan) | **DEAD** |
| `sessionIdAtom` | `state.ts` | nothing | **DEAD** |
| `rawOutputAtom` | `state.ts` | nothing | **DEAD** |

---

## 4. Identified Pain Points

### P1 — Dead code cluster (high priority)
5 components totalling ~880 lines (`SetupWizard`, `ConnectionStatus`, `AgentHeader`, `ProcessControls`, `InitializationPrompt`) are mounted nowhere and consume 7 atoms that are never written. This dead code:
- Inflates bundle size
- Creates confusion about which auth/process flow is actually used
- Makes `state.ts` look much larger than it needs to be

### P2 — `LogFn` defined 5 times
The exact same type alias `(tag: string, text: string, level?: ...) => void` is declared in:
`launch-flow.ts`, `useAgentControllerStatus.ts`, `useBookmarks.ts`, `useHistoryPagination.ts`, `useSessionDigest.ts`.
Should be a single export in `types.ts`.

### P3 — `agent-model.ts` is too large (505 lines, does too much)
It owns: atom creation delegation, launch orchestration, input dispatch, history triggers, resync logic, and provider switching. It is a grab-bag ViewModel that has grown beyond a coherent single responsibility.

### P4 — Launch flow is entangled with model
`launch-flow.ts` (272 lines) is called from `agent-model.ts` but defines its own `LogFn`, its own phase types, its own callback contracts. The boundary between the model and the flow is unclear — callers reach into the flow's internal state via callbacks.

### P5 — `types.ts` is 426 lines of mixed concerns
Contains: render types (`DocumentNode`, node subtypes), session types (`SessionStats`, `TurnTokens`), streaming types (`StreamingState`, `StreamEvent`), auth types (`AuthState`), provider types (`ProviderConfig`), command types, and UI state types — all in one file. Finding a type requires knowing the file exists.

### P6 — No provider abstraction at the command level
`commands/providers/claude.ts` and `commands/providers/codex.ts` exist but the system has no unified `IProviderCommands` interface — they're registered with different shapes and a manual switch in `dispatch.ts`.

### P7 — `AgentDocumentView.tsx` is 513 lines
It handles: virtual scroll logic, node rendering dispatch, search highlighting, filter application, bookmark overlay, and RAF-based scroll tracking — all in one component. The render-dispatch logic alone could be extracted.

### P8 — `useAgentStream.ts` handles both transport and UI concerns
The hook decodes base64, accumulates line buffers, parses JSON, drives the translator, drives the parser, manages the RAF flush, deduplicates nodes, and handles health events. The transport/buffering layer and the document-update layer are intermingled.

### P9 — `init-monitor.ts` uses fragile byte-pattern matching
`init-monitor.ts` (262 lines) detects auth prompts by scanning raw output for URL patterns and known strings. This is brittle to CLI version changes and duplicates detection logic that should come via structured events.

### P10 — No explicit module boundary
There is no barrel file strategy — `index.ts` re-exports a partial set, and most imports are deep relative paths (e.g., `../../providers/translator-factory`). Refactors require updating many import paths.

---

## 5. Prioritized Improvement Steps

### Step 1: Delete dead code cluster (P1) — 1–2 hrs
**Files to delete:**
- `components/SetupWizard.tsx` (437 lines)
- `components/ConnectionStatus.tsx` (192 lines)
- `components/AgentHeader.tsx`
- `components/ProcessControls.tsx`
- `components/InitializationPrompt.tsx` + `InitializationPrompt.scss`

**Atoms to remove from `state.ts`:**
`processAtom`, `messageRouterAtom`, `authAtom`, `userInfoAtom`, `providerConfigAtom`, `sessionIdAtom`, `rawOutputAtom`

**Also remove from `types.ts`:** `AgentProcessState`, `MessageRouterState`, `AuthState`, `UserInfo`, `ProviderConfig` (if only used by dead code — verify first)

**Risk:** Low. Components are not mounted. Confirm with a grep for JSX usage before deleting.

---

### Step 2: Centralize `LogFn` (P2) — 30 min
Add to `types.ts`:
```typescript
export type LogFn = (tag: string, text: string, level?: "info" | "error" | "warn") => void;
```
Remove the 5 local definitions and import from `types.ts` instead.

---

### Step 3: Split `types.ts` into domain modules (P5) — 2 hrs
Create:
- `types/document.ts` — `DocumentNode` and all node subtypes
- `types/stream.ts` — `StreamEvent`, `StreamingState`
- `types/session.ts` — `SessionStats`, `TurnTokens`, `TurnActive`
- `types/provider.ts` — `ProviderConfig`, auth-adjacent types
- `types/commands.ts` — command type contracts (or move to `commands/types.ts` already exists)
- `types/index.ts` — re-export all for backwards compat

This is purely additive; no logic changes.

---

### Step 4: Extract `NodeRenderer` from `AgentDocumentView` (P7) — 2–3 hrs
`AgentDocumentView.tsx` should own: virtual list, scroll management, filter application.
A new `NodeRenderer.tsx` should own: dispatch by `node.type` to leaf renderers.

This makes each piece independently testable and reduces the 513-line file to ~200 lines each.

---

### Step 5: Extract transport layer from `useAgentStream` (P8) — 2 hrs
Split into:
- `useRawOutput.ts` — WPS subscribe, base64 decode, line-buffer accumulate, emit complete lines
- `useAgentStream.ts` (trimmed) — consume lines from `useRawOutput`, drive translator + parser, manage RAF flush + dedup

The dedup logic (nodeIdSet, nodeIndexMap, documentVersion) stays in `useAgentStream` since it's document-aware.

---

### Step 6: Shrink `agent-model.ts` (P3) — 3 hrs
Extract three responsibilities:
1. **`useAgentLaunch.ts`** — wraps `launch-flow.ts`, owns launch state machine, exposes `launch()`, `retry()`, `cancel()`
2. **`useAgentInput.ts`** — owns submit handler, history navigation, multi-line state
3. `agent-model.ts` becomes thin: wires atoms, calls hooks, exposes the unified API surface

---

### Step 7: Unify provider command interface (P6) — 1–2 hrs
Define `IProviderCommands` interface in `commands/types.ts`:
```typescript
export interface IProviderCommands {
    readonly provider: string;
    getCommands(): SlashCommand[];
    handleCommand(name: string, args: string[], ctx: CommandContext): Promise<void>;
}
```
`claude.ts` and `codex.ts` implement it. `dispatch.ts` routes to the active provider's `handleCommand` without a switch.

---

### Step 8: Emit structured auth events from backend (P9) — 3–4 hrs (backend)
Instead of byte-scanning in `init-monitor.ts`, have `persistent.rs` detect auth state from the CLI's structured output and emit a WPS `agentauth` event. Frontend subscribes and renders the auth overlay without fragile pattern matching.

This is the most invasive change — defer until after P1–P7 are complete.

---

### Step 9: Add barrel file strategy (P10) — 1 hr
`index.ts` should export the complete public API:
- All types from `types/index.ts`
- `createAgentAtoms` from `state.ts`
- `useAgentStream` from `useAgentStream.ts`
- ViewModel class from `agent-model.ts`

Internal imports within the module can use relative paths; cross-module imports use the barrel.

---

### Step 10: Add integration test for stream dedup (P8 follow-up) — 2 hrs
The duplicate-node race (fixed in PR #404) had no test coverage. Add a test in `stream-parser.test.ts` (or a new `useAgentStream.test.ts`) that simulates:
1. History load completing mid-flight (bumps `documentVersion`)
2. Concurrent RAF flush with nodes in `pendingNew`
3. Asserts no duplicate IDs in final document

---

## 6. Suggested Order

| Priority | Step | Effort | Risk |
|----------|------|--------|------|
| 1 | Delete dead code cluster | 1–2 hrs | Low |
| 2 | Centralize `LogFn` | 30 min | Trivial |
| 3 | Split `types.ts` | 2 hrs | Low |
| 4 | Extract `NodeRenderer` | 2–3 hrs | Medium |
| 5 | Extract transport layer | 2 hrs | Medium |
| 6 | Shrink `agent-model.ts` | 3 hrs | Medium |
| 7 | Provider command interface | 1–2 hrs | Low |
| 8 | Structured auth events | 3–4 hrs | High |
| 9 | Barrel file strategy | 1 hr | Low |
| 10 | Stream dedup test | 2 hrs | Low |

Steps 1–3 are pure removals or renames — zero risk and immediate payoff. Steps 4–6 are the structural refactors that require coordination. Step 8 is the only backend change and should be a separate PR.

---

## 7. What NOT to Change

- **`stream-parser.ts`** — well-tested, single responsibility, clean API. Leave it alone.
- **`providers/claude-translator.ts`** — complex but correct. Only touch if adding a new event type.
- **`useAgentStream.ts` RAF batching** — the performance design (pending buffers + single RAF flush) is intentional. Don't simplify it.
- **`useHistoryPagination.ts`** — the `documentVersion` bump protocol is load-bearing for dedup correctness (PR #404). Any change here needs the dedup race in mind.

---

## 8. Quick Wins Checklist

- [ ] Delete 5 orphan components (~880 lines removed)
- [ ] Remove 7 dead atoms from `state.ts`
- [ ] Remove dead types from `types.ts` (verify usage first)
- [ ] Consolidate `LogFn` to one definition in `types.ts`
- [ ] Add `// @internal` JSDoc comment on `nodeIdSet`/`nodeIndexMap` in `useAgentStream.ts` to explain the dedup invariant for future maintainers
