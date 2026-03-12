# Dead Code Strip — AgentMux Pre-SolidJS Cleanup

**Version:** 0.1 — 2026-03-11
**Branch:** agenta/solidjs-migration
**Goal:** Remove all dead/inherited code before the SolidJS rewrite so we start clean.

---

## 1. What Gets Deleted and Why

### 1.1 VDom System

**Verdict: Delete entirely.**

- Inherited from WaveTerm (Command Line Inc, 2025)
- Rust backend: `vdom.rs` comment explicitly says "reflection-based component rendering engine is deferred" — the backend driver was never finished
- No AgentMux code or workflow uses it
- The `term:vdomblockid` meta key exists but nothing sets it in practice
- Frontend wiring (`vdom.tsx`, `vdom-model.tsx`, `vdom-utils.tsx`, `termVDom.tsx`) is live code paths that go nowhere useful

**Files to delete:**
```
frontend/app/view/vdom/vdom.tsx
frontend/app/view/vdom/vdom-model.tsx
frontend/app/view/vdom/vdom-utils.tsx
frontend/app/view/term/termVDom.tsx
agentmuxsrv-rs/src/backend/vdom.rs
```

**Files to clean (remove vdom references):**
```
frontend/app/view/term/term.tsx              — remove TermVDomNode, TermToolbarVDomNode imports/usage
frontend/app/view/term/termViewModel.ts      — remove vdomBlockId, vdomToolbarBlockId atoms, vdomMode logic
frontend/app/view/term/term-wsh.tsx          — remove vdom RPC handling
frontend/app/block/block.tsx                 — remove vdom view type registration
frontend/app/store/wshclientapi.ts           — remove VDomRender, VDomUrlRequest, VDomCreate* commands
frontend/types/gotypes.d.ts                  — remove VDomElem, VDomEvent, VDomBinding, VDomRef, etc.
agentmuxsrv-rs/src/backend/mod.rs           — remove vdom module
agentmuxsrv-rs/src/backend/apprunner.rs     — remove vdom imports/usage
agentmuxsrv-rs/src/server/mod.rs            — remove vdom RPC handlers
```

---

### 1.2 ijson.tsx

**Verdict: Delete entirely.**

- Uses `react-frame-component` to render JSON-described UI inside an iframe
- Never imported by any other file
- The `FileAppendIJson` RPC command exists but the renderer is dead

**Files to delete:**
```
frontend/app/view/term/ijson.tsx
```

---

### 1.3 Dead npm Packages

**Verdict: Remove from package.json.**

| Package | Evidence of death |
|---------|------------------|
| `react-resizable-panels` | Zero imports in entire frontend — layout is fully custom |
| `react-frame-component` | Only in `ijson.tsx` (itself deleted) |
| `react-hook-form` | Zero imports in entire frontend |
| `@table-nav/core` | Zero imports in entire frontend |
| `@table-nav/react` | Zero imports in entire frontend |
| `@react-hook/resize-observer` | Zero imports — layout uses ResizeObserver directly |
| `@radix-ui/react-label` | Check audit result |
| `@radix-ui/react-slot` | Check audit result |

---

### 1.4 Other Candidates (pending audit)

- `@floating-ui/react` — check if used directly or only transitively
- `@ai-sdk/react` — check if used in agent view or only installed
- `overlayscrollbars-react` — check if used or if base `overlayscrollbars` is used directly
- `@tanstack/react-table` — check if used in any view
- `use-device-pixel-ratio` — check usage
- `htl` — check usage
- `parse-srcset` — check usage
- `prop-types` — check usage (React-specific, likely dead)

---

## 2. What Does NOT Get Deleted

| Item | Reason to keep |
|------|---------------|
| `wshclientapi.ts` VDom RPC stubs | Keep the RPC method signatures for now — backend still declares them. Can strip later. |
| `term:vdom*` meta key types in `gotypes.d.ts` | Keep type definitions, just remove frontend wiring |
| `agentmuxsrv-rs/src/backend/vdom.rs` types | Backend may reference these in RPC protocol — audit first |
| `cssparser.rs` | Referenced by vdom.rs but may be used elsewhere — audit first |

---

## 3. Execution Order

1. **Audit** — confirm all references (agent running)
2. **Delete vdom frontend files** (`vdom/`, `termVDom.tsx`, `ijson.tsx`)
3. **Clean term files** — remove vdom imports, vdomBlockId atoms, vdom termMode branches
4. **Clean types** — strip VDom* type declarations from gotypes.d.ts
5. **Clean wshclientapi.ts** — remove VDom RPC commands
6. **Clean Rust backend** — remove vdom.rs, clean mod.rs/apprunner.rs/server/mod.rs
7. **Remove dead packages** — update package.json, run npm install
8. **TypeScript check** — `tsc --noEmit` to catch any missed references
9. **Cargo check** — verify Rust compiles

---

## 4. Risk Assessment

**Low risk** — all deletions are isolated systems with clear boundaries:
- VDom has its own directory (`view/vdom/`)
- The terminal vdom hooks are conditional on `term:vdomblockid` meta being set (never set in practice)
- No shared utilities between VDom and other views

**Watch for:**
- `vdom-utils.tsx` may contain utilities used outside VDom (check before deleting)
- `cssparser.rs` is referenced from `vdom.rs` — verify no other Rust code uses it
- Any shared types in `gotypes.d.ts` that happen to be in the VDom section but are used elsewhere
