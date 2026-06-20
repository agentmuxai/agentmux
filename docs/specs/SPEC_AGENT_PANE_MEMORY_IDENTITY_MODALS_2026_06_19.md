# Agent Pane Memory & Identity Modals

**Date:** 2026-06-19  
**Status:** Draft  
**Replaces:** `AgentFocusedPanel` / `AgentCardSettingsPanel` cog-overlay system

---

## 1. Motivation

The agent pane header currently has a single cog (⚙) icon that opens a half-pane overlay with two tabs — Agent settings and Identity. This design has three problems:

1. **No memory access from the pane.** The agent's native memory folder (`~/.claude/projects/.../memory/`) is invisible and uneditable from the UI.
2. **Wrong container.** The cog overlay is a custom half-pane overlay, not the project-wide modal system. Two containers means two visual languages and two code paths.
3. **Overloaded icon.** One icon surfaces two unrelated things (agent config and identity accounts).

This spec replaces the cog with two dedicated icons that open proper pane-scoped modals from the existing modal system:

| Icon | FontAwesome 6 | Opens |
|---|---|---|
| Brain | `fa-brain` (solid) | Native memory folder browser + editor |
| ID card | `fa-id-card` (solid) | Agent identity — accounts list |

---

## 2. What Gets Removed

- **`AgentFocusedPanel.tsx`** — the half-pane overlay wrapper. Deleted.
- **`AgentCardSettingsPanel.tsx`** — the tab panel (Agent / Identity tabs). Deleted.
- **The cog `endIconButton`** in `agent-model.ts`. Replaced with two icons.
- **`OverlayTab` type** (`"agent" | "identity"`). Replaced with modal open calls.
- **`_setOverlayTab` / `_lastOverlayTab`** on `AgentViewModel`. Removed.
- **`showOverlayTab` signal** in `agent-view.tsx`. Removed.

Agent definition settings (name, model, shell, etc.) remain editable via the agent definition list — the same `AgentDefDetail` / `AgentDefForm` flow that already exists there. They lose their pane-header shortcut, which is acceptable: agent config is infrequent; memory and identity are the live operational surfaces.

---

## 3. Two New Icons

### 3.1 `agent-model.ts`

```typescript
// OverlayTab type removed entirely.

this.endIconButtons = () => {
    const agentId = this.blockAtom()?.meta?.["agentId"];
    if (!agentId) return [];
    return [
        {
            elemtype: "iconbutton",
            icon: "brain",
            title: "Agent memory",
            click: () => { openMemoryModal(agentId); },
        },
        {
            elemtype: "iconbutton",
            icon: "id-card",
            title: "Agent identity",
            click: () => { openIdentityModal(agentId); },
        },
    ];
};
```

`openMemoryModal` and `openIdentityModal` are thin wrappers around the project-wide modal API (see §6).

---

## 4. Identity Modal

### 4.1 Content

Renders `AgentIdentityPanel` directly — no tabs, no header chrome beyond the modal's own title bar. The panel already exists and works; it just needs a new entry point.

```
┌─────────────────────────────────────────────────────┐
│  Identity — Parko                              [✕]  │
├─────────────────────────────────────────────────────┤
│  GitHub          [octokit-parko ▾]                  │
│  Anthropic       [parko-api-key ▾]                  │
│  AWS             [— none —       ▾]                 │
│  Google          [— none —       ▾]                 │
│  ...                                                 │
└─────────────────────────────────────────────────────┘
```

### 4.2 Implementation

- Reuse `AgentIdentityPanel` and `IdentityViewModel` unchanged.
- `handleAccountsUpdate` callback migrates from `AgentCardSettingsPanel` into the modal component.
- Modal width: ~480px, auto-height.

---

## 5. Memory Modal

### 5.1 Overview

Shows the agent's **native memory folder** — `~/.claude/projects/<sanitized-working-dir>/memory/` — as a two-column browser with a file list on the left and a read/edit view on the right.

The modal is labeled "Memory" and subtitled with the mirrored path so the user always knows where edits land on disk.

### 5.2 Path Computation

From the confirmed Claude Code source (`sanitizePath` in `sessionStoragePortable.ts`):

```
sanitized = path.replace(/[^a-zA-Z0-9]/g, '-')

if sanitized.length <= 200:
    project_dir = sanitized
else:
    project_dir = sanitized[0:200] + "-" + base36(djb2(path))
```

For memory specifically, Claude uses the **git repository root** (not cwd) when inside a git repo. For AgentMux agents the working directory IS the repo root (each agent has its own isolated dir), so `agent.working_directory` is the correct input.

Windows example:
```
C:\Users\area54\agentmux-agents\parko-0617i
→ C--Users-area54--agentmux-agents-parko-0617i
Memory dir: ~/.claude/projects/C--Users-area54--agentmux-agents-parko-0617i/memory/
```

Rust implementation:
```rust
fn sanitize_claude_path(path: &str) -> String {
    let s: String = path.chars().map(|c| {
        if c.is_ascii_alphanumeric() { c } else { '-' }
    }).collect();
    if s.len() <= 200 {
        return s;
    }
    let h = djb2_base36(path);
    format!("{}-{}", &s[..200], h)
}

fn djb2_base36(s: &str) -> String {
    let mut h: i32 = 0;
    for c in s.chars() {
        h = h.wrapping_shl(5).wrapping_sub(h).wrapping_add(c as i32);
    }
    radix_n((h as i64).unsigned_abs(), 36)
}
```

### 5.3 File Categories

| Filename | Role | Label in UI |
|---|---|---|
| `MEMORY.md` | Index file | "Index · loaded every session" |
| Any other `*.md` | Topic file | frontmatter `metadata.type` if present (`user` / `project` / `feedback` / `reference`), else "topic" |

### 5.4 Modal Layout

```
┌──────────────────────────────────────────────────────────────────┐
│  Memory — Parko                                           [✕]   │
│  ~/.claude/projects/C--Users-area54--agentmux-agents-parko-.../  │
│  memory/  ·  mirrored path — edits write directly to disk       │
├────────────────┬─────────────────────────────────────────────────┤
│ MEMORY.md  [i] │  # Memory Index                                 │
│ user_profile   │                                                  │
│ proj_agentmux  │  - [User profile](user_profile.md) — who...    │
│ proj_accounts  │  - [Project: AgentMux](project_agentmux.md)... │
│                │  - [Project: Accounts](project_accounts.md)...  │
│ + New file     │                                                  │
│                │                          [Edit]  [Save]         │
└────────────────┴─────────────────────────────────────────────────┘
```

- `[i]` tooltip on MEMORY.md: "This file is loaded into every new Claude session for this agent. Edits take effect on the next session start."
- Topic files: clicking opens content in right panel. Frontmatter rendered as a collapsible metadata block, body as editable textarea (raw markdown).
- **Edit mode:** textarea with Save / Cancel buttons. Save writes via `NativeMemoryWriteFile` RPC.
- **New file:** prompts for filename (auto-appends `.md` if omitted), creates empty file.
- Modal is wider than identity: ~780px with resizable split.

### 5.5 Empty State

If `~/.claude/projects/<sanitized>/memory/` doesn't exist or is empty:

```
No memory files yet.

Claude Code creates this folder when it first saves a memory for this agent.
You can also create files manually — they'll be available at the next session start.

[+ Create MEMORY.md]
```

The "Create MEMORY.md" button creates the directory + file with a starter template.

---

## 6. Modal API Integration

The agent pane already wraps its entire content in `<ModalLayer scope="pane">` (`agent-view.tsx` line 104). Any component inside the pane can call `useModalLayer()` to open a pane-scoped modal — it inerts and backdrops only the pane, not the whole window.

Pane-scoped modals are dispatched via a `ModalRequest` union (defined in `frontend/app/element/modal-layer.ts`). To add the two new modals, we extend the union with new `kind` values and add matching cases to the `ModalLayer` dispatcher.

```typescript
// frontend/app/element/modal-layer.ts  (additions)
export type ModalRequest =
    | { kind: "launch-agent"; agent: AgentDefinition }
    // ... existing kinds ...
    | { kind: "agent-memory";   agentId: string; workingDirectory: string }
    | { kind: "agent-identity"; agent: AgentDefinition }
```

```typescript
// In the ModalLayer dispatcher (ModalLayer.tsx):
// Add cases alongside existing "launch-agent" etc.
case "agent-memory":
    return <AgentNativeMemoryModal agentId={req.agentId} workingDirectory={req.workingDirectory} />;
case "agent-identity":
    return <AgentIdentityModal agent={req.agent} />;
```

Call site (inside agent icon button handlers — note `useModalLayer()` must be called in component context, so the clicks are wired through a reactive context or a stored ref):

```typescript
// agent-view.tsx — wire into AgentPresentationView where modalLayer is in scope
const modalLayer = useModalLayer();

// Brain icon click:
modalLayer.open({ kind: "agent-memory", agentId, workingDirectory: agent.working_directory });

// ID-card icon click:
modalLayer.open({ kind: "agent-identity", agent });
```

The `_setOverlayTab` callback pattern on `AgentViewModel` is removed entirely. The icon buttons' `click` handlers are registered as closures in the model but called by the pane view where the modal layer context exists (same pattern as `AgentPicker.tsx` → `buildLaunchRequest`).

---

## 7. New Backend RPCs

Three new commands registered in `app_api.rs`:

### `NativeMemoryListCommand`
```
Request:  { agent_id: String }
Response: { files: Vec<NativeMemoryFileMeta> }

NativeMemoryFileMeta {
    filename: String,       // e.g. "MEMORY.md", "user_profile.md"
    is_index: bool,         // true only for MEMORY.md
    metadata_type: Option<String>,  // parsed from YAML frontmatter: user/project/feedback/reference
    size_bytes: u64,
    modified_at: i64,       // unix ms
}
```

Behavior: compute memory dir from `agent.working_directory`, list `*.md` files. Returns empty vec (not error) if dir doesn't exist.

### `NativeMemoryReadFileCommand`
```
Request:  { agent_id: String, filename: String }
Response: { content: String }
```

Behavior: read the file. Returns error if filename contains path separators (security: no directory traversal).

### `NativeMemoryWriteFileCommand`
```
Request:  { agent_id: String, filename: String, content: String }
Response: {}
```

Behavior: create directory if needed, write file atomically (write to `.tmp` then rename). Validates filename (alphanumeric + `-_`, ends in `.md`, no path separators).

---

## 8. Frontend Model

New `AgentNativeMemoryModel` (class, same pattern as `MemoryViewModel`):

```typescript
class AgentNativeMemoryModel {
    agentId: string;
    filesAtom: Accessor<NativeMemoryFileMeta[]>;   // reactive file list
    selectedFilenameAtom: Accessor<string | null>;
    contentAtom: Accessor<string | null>;            // content of selected file
    editingAtom: Accessor<boolean>;
    draftContentAtom: Accessor<string>;
    savingAtom: Accessor<boolean>;
    errorAtom: Accessor<string | null>;

    loadFiles(): Promise<void>;
    selectFile(filename: string): Promise<void>;   // fetches content
    startEdit(): void;
    cancelEdit(): void;
    saveEdit(): Promise<void>;
    createFile(filename: string): Promise<void>;
}
```

---

## 9. Implementation Phases

### Phase 1 — Icons + remove overlay (no memory backend yet)
- `agent-model.ts`: replace cog with brain+id-card icons (no-op clicks for now)
- Delete `AgentFocusedPanel.tsx`, `AgentCardSettingsPanel.tsx`
- `agent-view.tsx`: remove overlay signal + `<AgentFocusedPanel>` render
- Wire identity icon → identity modal via existing modal system

### Phase 2 — Memory backend
- `native_memory_handlers.rs`: List / Read / Write RPCs
- Register in `app_api.rs`
- Type generation

### Phase 3 — Memory modal UI ✅ DONE
- `agent-native-memory-model.ts` — `AgentNativeMemoryModel` (list / select / read / edit / save / create)
- `AgentNativeMemoryModal.tsx` — two-column browser + editor; replaces the Phase 1 placeholder
- `AgentNativeMemoryModal.scss` — styles
- `modal-dispatch.tsx` — `agent-memory` case now renders `AgentNativeMemoryModal` (passes `agentId` + `agentName`)
- Old `AgentMemoryModal.tsx` placeholder deleted
- Empty-state "Create MEMORY.md" flow included (moved up from Phase 4)
- Frontend-side filename validation mirrors backend `validate_filename`

### Phase 4 — Polish
- Resizable split pane in memory modal
- Relative "modified" timestamps + size in the file list
- Backend RPC to return the resolved memory dir path (replace the frontend
  `previewMemoryPath` approximation, which diverges from disk past 200 chars)
- `autoMemoryEnabled` re-evaluation (§10)

---

## 10. Memory Sync Architecture (research-confirmed)

### 10.1 Revert `autoMemoryEnabled: false` — it was wrong

PR #1584 set `autoMemoryEnabled: false` to prevent Claude writing autonomously. Research
(GitHub issue #63903, open 2026-05-30) confirms this was the wrong call:

- `autoMemoryEnabled: false` suppresses WRITES only — Claude still cannot write facts
- The 11,300-token preamble (Sonnet) / 16,200-token preamble (Opus) loads regardless
- We pay the token cost with zero benefit

**Fix (follow-up PR):** Remove the `or_insert(json!(false))` line from `agent_config.rs`
`build_settings_with_hooks()`. Let `autoMemoryEnabled` default to `true` (the CLI default).
Claude writes facts to native memory; AgentMux surfaces them in the memory modal.

### 10.2 Concurrent write safety — already solved

GitHub issue #29051 documents a real concurrent-write corruption bug (no file locking,
non-atomic write = truncate then write). For AgentMux this is a **non-issue**:

Each AgentMux agent has its own isolated working directory
(`~/.agentmux-agents/<name>-<id>/`). The native memory path is derived from the
working directory. Isolated dirs → isolated memory paths → no concurrent writers
on the same MEMORY.md. No extra configuration needed.

### 10.3 Layer separation — enforced by content type

| Layer | Content | Written by | Survives compaction? |
|---|---|---|---|
| Global bundles → CLAUDE.md | Rules, policies, tool configs, team standards | Human (AgentMux injects at launch) | Yes — project-root CLAUDE.md auto-reloads post-compact |
| Native memory (`memory/*.md`) | Discovered facts, codebase patterns, session insights | Claude autonomously | MEMORY.md: treat as NO — use PostCompact hook. Topic files: on-demand, always available |

**The rule that prevents drift:** global bundles contain ONLY rules/policies (things that
don't change during a session). Native memory contains ONLY facts/discoveries (things
Claude learns). If Claude writes a rule to MEMORY.md, the user should review it quarterly
and either promote it to a bundle or delete it — never let both layers hold the same fact.

### 10.4 Compaction safety

Project-root `CLAUDE.md` (where global bundles are injected) **auto-reloads after
`/compact`** — no PreCompact hook needed for bundle content. The `InstructionsLoaded`
event with `load_reason: "compact"` fires to confirm re-injection happened.

MEMORY.md status post-compact is not explicitly guaranteed by docs. Add a
`SessionStart` hook with `matcher: "compact"` to re-read MEMORY.md if needed:

```json
{
  "hooks": {
    "SessionStart": [{
      "matcher": "compact",
      "hooks": [{ "type": "command", "command": "cat <memory-dir>/MEMORY.md" }]
    }]
  }
}
```

### 10.5 MEMORY.md discipline

- **200-line / 25KB hard cap** at load — beyond this is invisible at session start
- MEMORY.md must be an index only; all detail goes in topic files (loaded on demand)
- Topic files are never auto-loaded — Claude reads them when it judges they're relevant
- AgentMux memory modal shows both the index and all topic files so users can prune

### 10.6 Follow-up PR scope

1. Remove `autoMemoryEnabled: false` from `agent_config.rs` build_settings_with_hooks()
2. Update `agent-seed.json` if the setting was baked into any seed content
3. Update §10 of this spec to "complete" once shipped

---

## 11. Files Changed Summary

| File | Change |
|---|---|
| `frontend/app/view/agent/agent-model.ts` | Replace cog with 2 icons; remove OverlayTab, _setOverlayTab, _lastOverlayTab |
| `frontend/app/view/agent/agent-view.tsx` | Remove showOverlayTab signal + AgentFocusedPanel render |
| `frontend/app/view/agent/components/AgentFocusedPanel.tsx` | **DELETE** |
| `frontend/app/view/agent/components/AgentCardSettingsPanel.tsx` | **DELETE** |
| `frontend/app/view/agent/components/AgentNativeMemoryModal.tsx` | **NEW** |
| `frontend/app/view/agent/agent-native-memory-model.ts` | **NEW** |
| `agentmux-srv/src/server/native_memory_handlers.rs` | **NEW** |
| `agentmux-srv/src/server/app_api.rs` | Register 3 new RPCs |
| `agentmux-srv/src/bin/gen_types.rs` (or equivalent) | Add RPC type exports |
