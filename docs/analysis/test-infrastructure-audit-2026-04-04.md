# Test Infrastructure Audit — 2026-04-04

## Executive Summary

AgentMux has a **distributed test infrastructure** spanning three platforms:
- **Rust backend** (agentmux-srv): 907+ unit/integration tests
- **Frontend** (TypeScript/SolidJS): 10 vitest files testing UI and layout
- **E2E**: 2 Playwright tests — **stale**, written for removed Tauri host
- **No CI/CD**: No GitHub Actions workflows — tests only run manually

---

## What Works Today

| Component | Framework | Command | Tests |
|-----------|-----------|---------|-------|
| Rust unit tests | cargo test | `cargo test -p agentmux-srv` | 857 `#[test]` + 50 `#[tokio::test]` |
| Rust integration | cargo test | `cargo test -p agentmux-srv --test integration_test` | 4 tests |
| Frontend unit | Vitest 3.0.9 | `npm test` | 10 test files |
| Layout engine | Vitest | `npm test` | 4 test files (689 lines) |
| Coverage | Istanbul | `npm run coverage` | LCOV output to `./coverage/` |
| CDK infra | Jest | `cd infra/cdk && npm test` | 2 test files |

## What's Broken / Stale

| Component | Issue |
|-----------|-------|
| E2E Playwright tests | Written for Tauri — reference `_electron`, `tauri-driver`, removed paths |
| WebdriverIO configs | `wdio.conf.cjs` + `wdio.macos.conf.cjs` — target dead Tauri architecture |
| npm E2E scripts | `test:e2e`, `test:e2e:macos`, `test:e2e:debug` — all broken |
| CI/CD | No `.github/workflows/` — zero automated test runs on push/PR |
| Shell copytests | 48 bash scripts in `tests/copytests/cases/` — not integrated into any runner |

---

## Rust Test Breakdown (907+ tests)

### By Module (top files)

| File | Tests | Notes |
|------|-------|-------|
| `backend/ijson.rs` | 33 | JSON manipulation |
| `backend/dbutil.rs` | 26 | SQLite helpers |
| `backend/fileutil.rs` | 26 | File operations |
| `backend/base.rs` | 21 | Base utilities |
| `backend/envutil.rs` | 18 | Environment/path |
| `backend/wcore/mod.rs` | 18 | Core CRUD operations |
| `backend/ai/tools.rs` | 17+4 | AI tool use |
| `backend/ai/mod.rs` | 15 | AI routing |
| `backend/blockcontroller/shell.rs` | 16 | Shell controller lifecycle |
| `backend/blockcontroller/mod.rs` | 14 | Controller registry |
| `backend/wconfig/mod.rs` | 31 | Config parsing/serde |

### Integration Test (`agentmux-srv/tests/integration_test.rs`)

4 tests that spawn the real backend binary:
1. `health_returns_200` — HTTP health check
2. `auth_rejects_missing_key` — 401 on missing auth
3. `auth_accepts_valid_header` — 200 with valid auth
4. `sigterm_exits_process` — graceful shutdown

**Note:** Integration tests don't compile currently — `server/tests.rs` has pre-existing struct field mismatches.

### Untested Crates

- `agentmux-cef` — no tests (CEF host, IPC bridge)
- `agentmux-wsh` — no tests (shell integration CLI)
- `agentmux-launcher` — no tests (tiny DLL path launcher)

---

## Frontend Test Breakdown (10 files)

### Vitest Config (`vitest.config.ts`)
- Reporter: verbose + JUnit XML
- Coverage: Istanbul → LCOV
- No coverage thresholds defined

### Test Files

| File | Purpose |
|------|---------|
| `app/block/autotitle.test.ts` | Auto-title from workspace paths |
| `app/store/backendStatus.test.ts` | Backend connectivity state |
| `app/tab/tabbar-dnd.test.ts` | Tab bar drag-and-drop |
| `app/view/agent/providers/index.test.ts` | AI provider selection |
| `app/view/agent/state.test.ts` | Agent state management |
| `app/view/agent/stream-parser.test.ts` | Stream response parsing |
| `layout/tests/layoutModel.test.ts` | Layout model (196 lines) |
| `layout/tests/layoutNode.test.ts` | Layout nodes (301 lines) |
| `layout/tests/layoutTree.test.ts` | Layout tree (84 lines) |
| `layout/tests/utils.test.ts` | Layout utilities (108 lines) |

### Untested Frontend Areas
- Terminal view (`app/view/term/`) — no tests
- Forge view (`app/view/forge/`) — no tests
- Store layer (`app/store/wos.ts`, `global.ts`) — minimal coverage
- IPC layer (`app/platform/ipc.ts`) — no tests
- Most UI components — no tests

---

## E2E Tests (Stale)

### Files
- `e2e/close-button.test.ts` (112 lines) — Playwright + Electron API
- `e2e/widget-click.test.ts` (134 lines) — Playwright + CDP on port 9333
- `e2e/debug-launch.ts`, `test-agent-debug.ts`, `test-close-button.ts` — helper scripts

### Why Stale
- Reference `_electron as electron` API (Electron host removed)
- Reference `make/win-unpacked/AgentMux.exe` (Tauri build path, deleted)
- WebdriverIO configs target `tauri-driver` (removed)
- Port 9333 CDP was WebView2 debugging — CEF uses different mechanism

### Configs (all stale)
- `playwright.config.ts` — timeout 120s, workers 1, HTML reporter
- `wdio.conf.cjs` — targets `src-tauri/target/release/agentmux`
- `wdio.macos.conf.cjs` — mocks `window.__TAURI_INTERNALS__`

---

## Shell Integration Tests

48 bash scripts in `tests/copytests/cases/` (test000.sh through test047.sh).

Test `wsh file copy` operations with various scenarios. Not integrated into any standard test runner — must be run manually.

---

## Test Commands Reference

```bash
# Frontend unit tests
npm test                                    # vitest (watch mode)
npm run coverage                            # vitest + istanbul coverage

# Rust tests
cargo test -p agentmux-srv                  # all unit + integration
cargo test -p agentmux-srv --lib            # unit tests only
cargo test -p agentmux-srv -- wcore         # filter by name

# CDK tests
cd infra/cdk && npm test                    # jest

# E2E (STALE — do not run)
# npm run test:e2e                          # broken (Tauri)
# npm run test:e2e:macos                    # broken (Tauri)
```

---

## File Tree

```
agentmux/
├── vitest.config.ts                    # Frontend test config
├── playwright.config.ts                # E2E config (stale)
├── wdio.conf.cjs                       # WebdriverIO (stale)
├── wdio.macos.conf.cjs                 # WebdriverIO macOS (stale)
│
├── e2e/                                # E2E tests (stale)
│   ├── close-button.test.ts
│   └── widget-click.test.ts
│
├── frontend/
│   ├── app/
│   │   ├── block/autotitle.test.ts
│   │   ├── store/backendStatus.test.ts
│   │   ├── tab/tabbar-dnd.test.ts
│   │   └── view/agent/
│   │       ├── providers/index.test.ts
│   │       ├── state.test.ts
│   │       └── stream-parser.test.ts
│   └── layout/tests/
│       ├── layoutModel.test.ts
│       ├── layoutNode.test.ts
│       ├── layoutTree.test.ts
│       └── utils.test.ts
│
├── agentmux-srv/
│   ├── src/                            # 63 modules with #[cfg(test)]
│   │   └── (857 #[test] + 50 #[tokio::test])
│   └── tests/
│       └── integration_test.rs         # 4 integration tests
│
├── infra/cdk/test/
│   ├── agentmux-webhook-stack.test.ts
│   └── integration.test.ts
│
└── tests/copytests/cases/
    └── test000.sh .. test047.sh        # 48 shell scripts
```

---

## Critical Gaps

1. **No CI/CD** — zero automated test runs on push/PR
2. **E2E tests stale** — written for removed Tauri host, need CEF rewrite
3. **No coverage enforcement** — thresholds not set
4. **Integration tests broken** — `server/tests.rs` has pre-existing compile errors
5. **agentmux-cef/wsh untested** — lightweight but no test coverage at all
6. **48 shell copytests orphaned** — not in any runner

## Recommendations

1. **GitHub Actions**: `cargo test` on Rust changes, `npm test` on frontend changes
2. **Fix integration tests**: Update `server/tests.rs` struct fields
3. **Delete or rewrite E2E**: Remove stale Tauri tests, write CEF equivalents if needed
4. **Coverage gates**: Set 70% floor on frontend, enforce in CI
5. **Integrate copytests**: Either wrap in cargo test harness or remove
