> **Archived 2026-07-19:** Session handoff — both PRs described here (#1584, #1585) merged long ago. "Trust Center" is this UI's pre-rename name (renamed to Armory, PR #1917, 2026-07-02) — left as written for historical accuracy. Kept for reference only.

# Handoff — Memory/Identity Modal Redesign
**Date:** 2026-06-19  
**Branch:** `agenta/feat-memory-identity-modals`  
**Head:** `c430bff1`  
**Author:** AgentA  

---

## What was completed this session

### PR #1584 — Global memory bundles (MERGED `d928b384`)
Added `is_global` flag to memory bundles in the Trust Center. Bundles with `is_global=true` are injected into every agent's `CLAUDE.md` at launch. This is **Layer 1** of the two-layer memory model: human-defined rules/policies that Claude reads but doesn't write.

### PR #1585 — Re-enable `autoMemoryEnabled` (MERGED `2c0ade87`)
Reverted a prior `or_insert(json!(false))` in `agentmux-srv/src/backend/agent_config.rs` that was suppressing native memory writes. Research (GitHub issue #63903) confirmed the 11k-token preamble loads regardless — disabling writes had zero benefit. 

**Key fix in `agentmux-srv/src/server/app_api.rs`** (`write_agent_config_files`): instead of disabling globally, we only disable `autoMemoryEnabled` for agents using the *shared* slug-based workdir (`~/.agentmux/agents/<slug>`), because concurrent MEMORY.md writes there risk corruption (GitHub issue #29051). Agents with an explicit `working_directory` (isolated dirs) keep writes enabled.

### PR #1587 — Brain + id-card icons replace cog overlay (OPEN, awaiting bot reviews)
**This is where you resume.** Phase 1 of `SPEC_AGENT_PANE_MEMORY_IDENTITY_MODALS_2026_06_19.md`.

---

## PR #1587 — What changed

### Deleted
- `frontend/app/view/agent/components/AgentFocusedPanel.tsx`
- `frontend/app/view/agent/components/AgentCardSettingsPanel.tsx`

Both were the half-pane overlay system (cog → Agent/Identity tabs). Gone entirely.

### Modified — `frontend/app/view/agent/agent-model.ts`
- Removed `export type OverlayTab = "agent" | "identity"`
- Removed `_setOverlayTab: ((tab: OverlayTab | null) => void) | null`
- Removed `_lastOverlayTab: OverlayTab = "agent"`
- Added `_openIdentityModal: (() => void) | null = null`
- Added `_openMemoryModal: (() => void) | null = null`
- `endIconButtons` now returns `["brain", "id-card"]` icons instead of single `"gear"`

### Modified — `frontend/app/view/agent/agent-view.tsx`
- Removed `import { AgentFocusedPanel }` (line was 69)
- Added `import { useModalLayer } from "@/element/modal-layer"`
- Removed `showOverlayTab` signal + `_setOverlayTab` mount/cleanup wiring
- Added `useModalLayer()` call + wires `_openIdentityModal` and `_openMemoryModal` on mount/cleanup
- Removed `<AgentFocusedPanel ...>` render block (was ~1011-1020)

### Modified — `frontend/app/element/modal-layer.ts`
Added to `ModalLayerRequest` union:
```typescript
| AgentIdentityRequest  // { kind: "agent-identity"; agent: AgentDefinition; blockId: string }
| AgentMemoryRequest    // { kind: "agent-memory"; agentId: string; agentName: string; workingDirectory: string }
```

### Modified — `frontend/app/element/modal-dispatch.tsx`
Added imports + cases for both new kinds. Dispatch:
- `"agent-identity"` → `<AgentIdentityModalPanel agent blockId onClose={api.close} />`
- `"agent-memory"` → `<AgentMemoryModalPanel agentName workingDirectory onClose={api.close} />`

### New — `frontend/app/view/agent/components/AgentIdentityModal.tsx`
Full implementation. Wraps `AgentIdentityPanel` with a "Done" button. `IdentityViewModel` instantiated with `(blockId, null as any)` — nodeModel is stored but never called in the constructor so null is safe. `handleAccountsUpdate` sends `UpdateAgentDefinitionCommand` with all existing agent fields + new `accounts`.

### New — `frontend/app/view/agent/components/AgentMemoryModal.tsx`
Phase 1 placeholder. Shows `previewMemoryPath(workingDirectory)` (client-side approximation of the sanitize algorithm: replace non-alphanumeric with `-`, truncate at 48 chars for display). Full browser in Phase 3.

---

## Architecture — Two-Layer Memory Model

| Layer | Content | Written by | Where |
|---|---|---|---|
| 1 — Global bundles | Rules, policies, tool configs | Human (via Trust Center) | Injected into agent's `CLAUDE.md` at launch via `is_global` flag |
| 2 — Native memory | Discovered facts, codebase patterns, session insights | Claude autonomously | `~/.claude/projects/<sanitized-workdir>/memory/` |

**Path computation** (Claude Code `sessionStoragePortable.ts`):
```
sanitized = path.replace(/[^a-zA-Z0-9]/g, '-')
if len <= 200: project_dir = sanitized
else: project_dir = sanitized[0:200] + "-" + base36(djb2(path))
```
Windows example: `C:\Users\area54\agentmux-agents\parko-0617i` → `C--Users-area54--agentmux-agents-parko-0617i`

**MEMORY.md** is the index file (auto-loaded every session, 200-line/25KB hard cap). Topic `.md` files have YAML frontmatter (`name`, `description`, `metadata.type: user|project|feedback|reference`) and are loaded on demand.

---

## What to do when you wake up

### Step 1 — Merge PR #1587
```bash
gh api repos/agentmuxai/agentmux/pulls/1587/reviews \
  --jq '.[] | "\(.user.login) [\(.state)]: \(.body[:120])"'
```
Read reagent review. Address any CHANGES_REQUESTED. Codex reviews on PR open — re-trigger with a5af PAT if it hasn't posted. Merge when reagent APPROVED on HEAD.

### Step 2 — Phase 2: Rust backend RPCs
New file: `agentmux-srv/src/server/native_memory_handlers.rs`

Three handlers:
```rust
// NativeMemoryListCommand
// Request: { agent_id: String }
// Response: { files: Vec<NativeMemoryFileMeta> }
// - Compute memory dir: get agent's working_directory, apply sanitize_claude_path(),
//   append "/.claude/projects/<sanitized>/memory/"
// - List *.md files; return empty vec (not error) if dir missing

// NativeMemoryReadFileCommand  
// Request: { agent_id: String, filename: String }
// Response: { content: String }
// - Reject filenames with path separators (no traversal)

// NativeMemoryWriteFileCommand
// Request: { agent_id: String, filename: String, content: String }
// Response: {}
// - Validate filename: only [a-zA-Z0-9_-] + ".md" suffix, no separators
// - Create dir if needed; write atomically (write to .tmp then rename)
```

Rust sanitize helper (goes in `native_memory_handlers.rs` or a shared util):
```rust
fn sanitize_claude_path(path: &str) -> String {
    let s: String = path.chars().map(|c| {
        if c.is_ascii_alphanumeric() { c } else { '-' }
    }).collect();
    if s.len() <= 200 { return s; }
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

Register in `agentmux-srv/src/server/app_api.rs` alongside other handlers, then run type generation.

### Step 3 — Phase 3: Full memory modal UI
Replace the placeholder in `AgentMemoryModal.tsx` with a real two-column browser:
- Left: file list (`MEMORY.md` + topic files, labeled by frontmatter `metadata.type`)
- Right: read/edit view (textarea with Save/Cancel)
- "New file" button → prompt filename → `NativeMemoryWriteFileCommand`
- Empty state: "No memory files yet" + "Create MEMORY.md" button

New class `AgentNativeMemoryModel` (same pattern as `MemoryViewModel`) with reactive atoms for files list, selected file, content, editing state, saving state, error state.

### Step 4 — Phase 4: Polish
- YAML frontmatter parsing for type labels
- Resizable split pane in memory modal (~780px wide, per spec §5.4)
- `[i]` tooltip on MEMORY.md: "Loaded into every new session"
- Empty-state "Create MEMORY.md" scaffold

---

## Key file paths

| File | Notes |
|---|---|
| `docs/specs/SPEC_AGENT_PANE_MEMORY_IDENTITY_MODALS_2026_06_19.md` | Full spec; §4 = identity modal, §5 = memory modal, §7 = backend RPCs, §8 = frontend model, §9 = phases |
| `frontend/app/element/modal-layer.ts` | `ModalLayerRequest` union — add new kinds here |
| `frontend/app/element/modal-dispatch.tsx` | Add `requestLabel` + `renderRequest` cases here |
| `frontend/app/view/agent/components/AgentIdentityModal.tsx` | NEW (Phase 1) — identity modal |
| `frontend/app/view/agent/components/AgentMemoryModal.tsx` | NEW (Phase 1) — memory stub |
| `frontend/app/view/agent/agent-model.ts` | `_openIdentityModal`, `_openMemoryModal` callbacks |
| `frontend/app/view/agent/agent-view.tsx` | Wires modal callbacks; `<ModalLayer scope="pane">` at line ~103 |
| `agentmux-srv/src/server/app_api.rs` | `write_agent_config_files` has the shared-workdir guard (~line 2297) |
| `agentmux-srv/src/backend/agent_config.rs` | Global settings injected at launch; `autoMemoryEnabled` now NOT suppressed |

---

## PR / merge sequence to complete this feature

| PR | Status | Content |
|---|---|---|
| #1584 | MERGED | Global memory bundles (`is_global` flag) |
| #1585 | MERGED | Re-enable `autoMemoryEnabled` + shared-workdir guard |
| #1587 | OPEN | Phase 1: brain+id-card icons + identity modal + memory stub |
| TBD | not started | Phase 2: Rust native memory RPCs |
| TBD | not started | Phase 3: Full memory modal UI |

---

## PR rules (from memory)

- **Never push to main** — all changes via PR
- **Read both reagent AND codex** before merging — `gh api repos/agentmuxai/agentmux/pulls/N/reviews` AND `gh api repos/agentmuxai/agentmux/issues/N/comments`
- codex only auto-reviews on PR open; re-trigger with `@codex review` comment using the a5af PAT (see `reference_a5af_pat_path.md`)
- reagent re-reviews on every push automatically
- Merge when reagent APPROVED on HEAD; codex quota-limit is acceptable blocker to skip
- `bump patch` before each build; use `scripts/bump-wrapper.sh` or sync package-lock.json after
