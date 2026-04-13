# Spec: Dead Code Removal & Treeshake

**Date:** 2026-04-06
**Branch:** `agenty/dead-code-removal` (or assigned agent)
**Closes:** Identified in OpenClaw/Anthropic policy analysis (agenty-workspace/openclaw-anthropic-report.md)

---

## Motivation

Two confirmed-dead code clusters exist in the codebase:

1. `agentmux-srv/src/backend/ai/` — a full AI provider abstraction layer (5 Rust files) that was ported from Go but never wired into the Rust backend. References "the Tauri app" in comments — Tauri was removed. The module is declared in `backend/mod.rs` but no RPC handler, service, or other module imports any type from it.

2. `frontend/app/view/agent/api-client.ts` — a direct Claude API client (257 lines) that is imported by zero files. Calls `https://api.anthropic.com/v1/messages` with an `x-api-key`. Dead proof-of-concept.

Beyond these two clusters, a broader treeshake pass is warranted: `agentmux-srv/src/main.rs` carries `#![allow(dead_code)]` at the crate level, which suppresses all dead-code warnings from the partial Go port. This makes it impossible to know what else is unused. This spec addresses that too.

**Secondary motivation:** `api-client.ts` calls the Anthropic API directly in a way that would violate Anthropic's ToS if it were ever activated with subscription OAuth credentials. Removing it eliminates that risk entirely, even if the code is currently inert.

---

## Scope

### 1. Remove `agentmux-srv/src/backend/ai/` (entire directory)

**Files to delete:**
```
agentmux-srv/src/backend/ai/mod.rs        (587 lines — types, traits, tests)
agentmux-srv/src/backend/ai/anthropic.rs  (stub, not implemented, references "Tauri app")
agentmux-srv/src/backend/ai/openai.rs     (stub, not implemented)
agentmux-srv/src/backend/ai/tools.rs      (tool execution framework)
agentmux-srv/src/backend/ai/chatstore.rs  (in-memory chat store)
```

**Files to update:**
```
agentmux-srv/src/backend/mod.rs  — remove line: pub mod ai;
```

**Evidence it is safe to delete:**
- Grep for `use.*backend::ai` or `backend::ai::` across all `.rs` files: zero results
- Grep for `AIOptsType`, `AIBackend`, `AIStreamRequest`, `select_backend`: zero results outside `ai/`
- The module `#[cfg(test)]` block in `mod.rs` only tests its own types — no integration tests reference it
- `anthropic.rs` comment: *"Actual HTTP streaming requires the reqwest crate which will be added when the AI feature is fully wired into the Tauri app"* — Tauri is gone; this code was never completed

**Notable detail:** `mod.rs` defines `DEFAULT_AI_ENDPOINT: "https://cfapi.agentmux.ai/api/waveai"` — a planned AgentMux cloud AI proxy. This design intent (managed proxy → separate billing) was the right architecture, but the implementation was never finished. If this feature is ever revisited, it should be built fresh against the current Rust/RPC architecture, not resurrected from this Go port stub.

**Compile verification:** After deletion, run `cargo build -p agentmux-srv` to confirm no references remain.

---

### 2. Remove `frontend/app/view/agent/api-client.ts`

**File to delete:**
```
frontend/app/view/agent/api-client.ts  (257 lines)
```

**Evidence it is safe to delete:**
- Grep for `from.*api-client` across all `.ts` / `.tsx` files in `frontend/`: zero results
- Grep for `ClaudeCodeApiClient`, `ClaudeCodeConfig`, `Conversation` (from this file): zero results outside the file itself
- File exports are never consumed anywhere in the build

**What the file does (for the record):**
- Implements a full Claude streaming API client calling `https://api.anthropic.com/v1/messages`
- Takes an `apiKey` parameter (API key, not OAuth — correct auth type)
- Converts Anthropic SSE stream format to the internal `StreamEvent` format
- Was apparently a PoC for a cloud-agent mode before the CLI subprocess approach was chosen

**Compile/lint verification:** After deletion, run `tsc --noEmit` and `npm run build:dev` to confirm no dangling imports.

---

### 3. Treeshake: Lift `#![allow(dead_code)]` from crate root

**File:** `agentmux-srv/src/main.rs` (or `lib.rs` — wherever the crate-level pragma lives)

The current `#![allow(dead_code)]` suppresses warnings globally, making it impossible to see what's actually unused. This was added to silence warnings from the partial Go port, but it also hides any new dead code introduced since.

**Approach:**
1. Remove `#![allow(dead_code)]` from the crate root
2. Run `cargo build -p agentmux-srv 2>&1 | grep "dead_code\|unused"` to collect all warnings
3. For each warning, classify:
   - **Delete**: If the item has no plausible near-term use and is from the Go port
   - **Tag with `#[allow(dead_code)]`**: If the item is intentionally unused but expected to be needed (e.g., scaffolding for a feature in active spec)
   - **Wire up**: If the item should be connected but isn't yet

4. The goal is to restore per-item dead-code suppression (or none at all) rather than blanket crate-level suppression.

**Note on scope:** This step is expected to surface a significant list of unused items from the Go port. The assignee should batch them into a second PR rather than trying to delete everything in one commit. The first PR (Steps 1 and 2 above) is safe and small. The treeshake is iterative.

---

## What to Keep

These were audited and confirmed in use:

| Item | Location | Used by |
|------|----------|---------|
| `register_backend_window` | `agentmux-cef/src/commands/window.rs:363` | `frontend/util/cef-api.ts:401` — called during window init to populate `window_id_map` for shell cleanup in `on_before_close` |
| `providers/claude-translator.ts` | `frontend/app/view/agent/providers/` | `translator-factory.ts` |
| `providers/codex-translator.ts` | same | `translator-factory.ts` |
| `providers/gemini-translator.ts` | same | `translator-factory.ts` |
| All other agent view files | `frontend/app/view/agent/` | Actively imported and used |

---

## Implementation Order

```
PR 1 (small, safe, no risk):
  - Delete agentmux-srv/src/backend/ai/ (5 files)
  - Remove pub mod ai; from backend/mod.rs
  - Delete frontend/app/view/agent/api-client.ts
  - cargo build + tsc --noEmit to confirm clean
  - bump patch

PR 2 (iterative, larger):
  - Remove #![allow(dead_code)] from crate root
  - Collect and triage dead-code warnings
  - Delete confirmed-dead Go port stubs in batches
  - Per-item #[allow(dead_code)] for intentional scaffolding
  - bump patch per batch
```

---

## Testing

**After PR 1:**
- `cargo build -p agentmux-srv` — must compile clean
- `tsc --noEmit` in `frontend/` — must have no new errors
- `task dev` — app launches, agent pane works (Claude Code CLI launch, auth, message send)
- `task cef:package:portable` — portable build completes

**After PR 2:**
- Same build/launch checks
- Confirm `cargo build` produces zero `dead_code` warnings (or only intentional `#[allow]`-annotated ones)

---

## Out of Scope

- `agenty/spec-openclaw-agent-runtime` — needs a separate policy review before any implementation decisions
- Any new AI feature work — this spec is removal only, not replacement
- Frontend treeshake beyond `api-client.ts` — no other orphaned frontend files were found in the audit
