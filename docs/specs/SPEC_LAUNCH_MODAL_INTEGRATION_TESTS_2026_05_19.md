# SPEC: Launch Modal — Integration Tests (jsdom-based)

**Status:** Draft
**Date:** 2026-05-19
**Author:** AgentA
**Related:**
- [SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19.md](./SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19.md) §6.10 — calls for an integration test that pins the memory-change-resets-auth regression.
- `frontend/app/store/launch-flow-state/` — the reducer slice the tests cross-verify against the view.

---

## 0. TL;DR

§6.10 of the launch-modal spec called for a "Playwright or equivalent" integration test that replays the memory-change-resets-auth repro. This spec picks **"or equivalent"**: use **Vitest + `@solidjs/testing-library` + jsdom** to mount `AgentLaunchModalPanel` with mocked RPCs and drive the flow programmatically. Catches the exact regression the spec wants pinned, runs in `npm test`, ~80–150 LOC, no separate CEF-spawn infrastructure.

Full-stack Playwright/WebDriver against the real CEF host is **out of scope here** — see §6 for why and what it would actually cost.

---

## 1. Why not Playwright/WebDriver?

The current repo has zero working e2e infrastructure:

- **No `@playwright/*` dep** in `package.json`.
- `test/specs/*.e2e.js` are **WebdriverIO + Tauri-helpers** files left over from before the Tauri-to-CEF migration. The helper file is literally named `tauri-helpers.js`. These tests have not run successfully against current `main` in months.
- `testdriver/` is an AI-driven UI tester for onboarding flows — different tool, narrow scope, not suitable for state-machine assertions.
- The only working test runner is **Vitest** (53 reducer-slice unit tests pass).

Standing up real Playwright against the CEF host needs all of:
1. CEF host process spawn fixture (build artifact path, ipc port discovery, ready-wait, teardown).
2. Frontend devtools-protocol attachment (CEF supports DevTools — port allocation + WebDriver bridge).
3. Mocks or fixtures for the `agentmux-srv` sidecar (else every test hits real RPC).
4. Per-test data-dir isolation (so tests don't poison each other).
5. CI runner with display server (Windows GH runners do; macOS/Linux need Xvfb or equivalent).

That's a 1-2 week build-out. The **regression we actually want to pin** — "memory change shouldn't unmount the auth panel" — is a frontend-only state-machine concern. jsdom mounts the component, dispatches the same commands the real user would, and asserts the same DOM. No CEF needed.

---

## 2. Library choice

| Library | Why |
|---|---|
| **Vitest** | Already the project's test runner. `vite.config.ts` + `vitest.config.ts` exist and work. CI runs `npm test`. |
| **jsdom** | Solid component testing's canonical DOM environment. Lightweight, fast, deterministic. (`happy-dom` is faster but less compatible; not worth the deviation.) |
| **`@solidjs/testing-library`** | Idiomatic mounting + cleanup for SolidJS components. Wraps `render(() => <Component />)` and auto-disposes between tests. ([docs](https://testing-library.com/docs/solid-testing-library/intro/)) |
| **`@testing-library/user-event`** | Realistic user interactions (type, click, select). Use `userEvent.setup()` before `render`. |
| **`@testing-library/jest-dom`** | Adds matchers like `toBeInTheDocument`, `toBeDisabled`. Plays with Vitest via `expect.extend`. |
| **`vi.mock`** | RPC mocking at the module boundary (`@/app/store/rpc-api`). |

These mirror the SolidJS docs' recommended stack ([guides/testing](https://docs.solidjs.com/guides/testing)).

---

## 3. Test surface

### 3.1 What we mock

| Module | Why | How |
|---|---|---|
| `@/app/store/rpc-api` | RPCs hit a real WebSocket — can't run in jsdom. | `vi.mock` the module; replace each `RpcApi.X` with a `vi.fn()` returning controllable promises. |
| `@/app/store/wps` (`waveEventSubscribe`) | Cross-tab push events come over the IPC pipe. | `vi.mock` to return a no-op `unsub`. Tests that need to drive push events expose a `triggerEvent(eventType, payload)` helper. |
| `@/app/platform/ipc` (`invokeCommand`) | OpenExternal / browser_pane_auth_* don't exist outside CEF. | `vi.mock` to no-op `vi.fn()`. |
| `@/app/store/global` (`getApi`) | `openExternal` on the catalog page. | `vi.fn()`. |

### 3.2 What we DON'T mock

- The `launch-flow-state` reducer + store — they're the SUT.
- The `AuthFlowController` — we want its state to flow through real dispatch.
- `solid-js` itself.

### 3.3 What we render

`AgentLaunchModalPanel` directly — bypasses `TabModalLayer` (which mounts the modal globally). The panel's props (`agent`, `onSubmit`, `onCancel`, `onRequestNewIdentity`, `onRequestNewMemory`, `initialFormState`) are easy to fixture.

---

## 4. Test plan

Each test sets up:
```ts
beforeEach(() => {
    vi.clearAllMocks();
    // Default identities list with one bound Claude identity
    vi.mocked(RpcApi.ListIdentityBundlesCommand).mockResolvedValue([
        { id: "ident-work", name: "Work", ...timestamps },
    ]);
    vi.mocked(RpcApi.ListMemoriesCommand).mockResolvedValue([
        { id: "mem-notes", name: "Notes", ...timestamps },
    ]);
    vi.mocked(RpcApi.ListIdentityBindingsCommand).mockResolvedValue([
        { identity_id: "ident-work", provider: "claude", account_id: "acc-1" },
    ]);
    vi.mocked(RpcApi.ListNamedAgentsCommand).mockResolvedValue([]);
});
```

### 4.1 Regression test (the §6.10 must-have)

```ts
it("changing Memory does not reset auth state to Connect", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    render(() => (
        <AgentLaunchModalPanel
            agent={claudeAgent}
            onSubmit={onSubmit}
            onCancel={() => {}}
        />
    ));

    // Wait for the initial loads to settle + auto-pick to run.
    await screen.findByRole("combobox", { name: /identity bundle/i });

    // With "Work" auto-picked + binding for "claude" present, the
    // Connect panel should NOT be visible — Launch should be enabled
    // once a name is typed.
    expect(screen.queryByText(/Connect with Claude/i)).not.toBeInTheDocument();

    await user.type(screen.getByLabelText(/agent name/i), "alpha");
    const launchBtn = screen.getByRole("button", { name: /launch/i });
    expect(launchBtn).not.toBeDisabled();

    // The regression repro: change Memory selection.
    await user.selectOptions(
        screen.getByRole("combobox", { name: /memory bundle/i }),
        "mem-notes",
    );

    // Assertion: auth state stays ready. Connect panel doesn't reappear.
    // Launch button stays enabled. The exact bug pinned.
    expect(screen.queryByText(/Connect with Claude/i)).not.toBeInTheDocument();
    expect(launchBtn).not.toBeDisabled();
});
```

### 4.2 Adjacent regression tests

| Scenario | Assertion |
|---|---|
| Form mounts with no identity selected, identities list returns a non-blank bundle | Auto-pick fires; dropdown shows that bundle selected; binding fetch fires; Launch enables once name typed. |
| Form mounts via continuation with legacy `identity_id="blank"` | Identity dropdown is editable (not locked); Launch disabled until user picks. |
| User types name, picks Identity, picks Memory, clicks Launch | `onSubmit` called with the right payload (instanceName, identityId, memoryId, …). |
| Backend emits `identitybundlebindings:changed:ident-work` mid-modal | View refetches and dispatches `BindingsChanged`; selector updates. |
| User clicks "+ New Identity" with current Memory selection | Picker callback receives the live snapshot (name, runtime, image, identityId, memoryId) — codex's PR #910 round 6/7 finding pinned. |
| OAuth success path: panel reaches `kind: "ready"` → `identityId` swaps to the new bundle, `loadIdentities` re-runs | Dropdown shows the new bundle; Connect panel goes away. |

Goal: **6–8 focused tests, ~250 LOC total**. Each test is one specific regression or one specific happy path.

### 4.3 What we deliberately don't test in this suite

- Full visual rendering / CSS — that's what manual smoke + Storybook (if we ever add it) catches.
- Real RPC round-trips — that's what unit tests for the backend handlers catch.
- Multi-window / pane integration — out of scope for jsdom.
- Performance / large-list rendering — separate concern.

---

## 5. Implementation plan

1. Add deps:
   ```
   npm i -D @solidjs/testing-library @testing-library/user-event @testing-library/jest-dom jsdom
   ```
2. Update `vitest.config.ts`:
   ```ts
   test: {
       environment: "jsdom",
       setupFiles: ["./test/vitest-setup.ts"],
       ...
   }
   ```
3. Create `test/vitest-setup.ts`:
   ```ts
   import "@testing-library/jest-dom/vitest";
   ```
4. Create `frontend/app/view/agent/components/AgentLaunchModal.integration.test.tsx` with the suite from §4.
5. Wire CI: `npm test` already runs Vitest — no new pipeline.

---

## 6. Out of scope (deferred)

### 6.1 Real Playwright/WebDriver against CEF

If we want full end-to-end coverage (real CEF + real srv + real DOM events), the path is:
1. Add `@playwright/test`.
2. Write a CEF launcher fixture: spawn `agentmux-portable/agentmux.exe` with a temp data dir, capture the dev-tools port from stdout, connect Playwright via `chromium.connectOverCDP`.
3. Stub the srv sidecar OR run it with a clean SQLite for each test.
4. Implement per-test isolation: data dir per worker, port allocation, cleanup.
5. CI: Windows runner already supports it; Linux needs Xvfb; macOS needs xvfb-run-equivalent or headless framework.

This is a 1-2 week build-out and gates by it: separate PR, separate spec. Not blocking Stage 2 completion.

### 6.2 Visual regression / screenshot diffing

Not enabled today. Would need Percy / Chromatic / Playwright snapshot. Separate concern.

### 6.3 The `recordDispatch` audit ring (§6.8 of the launch-modal spec)

Wiring `launch-flow-store` through `command-source.ts` so transitions show in the diag panel. Not test infrastructure — separate, mechanical follow-up.

---

## 7. Best-practice references

- [SolidJS testing guide](https://docs.solidjs.com/guides/testing) — canonical setup steps, why `solid-js` must be loaded once (Vite/Node duplication trap).
- [`@solidjs/testing-library` npm](https://www.npmjs.com/package/@solidjs/testing-library) — `render` API, `testEffect` for reactivity assertions.
- [Vitest browser/component testing](https://vitest.dev/guide/browser/component-testing) — alternative real-browser mode if jsdom limitations bite (e.g. layout-dependent assertions); not needed for this suite.
- [Testing Library — "tests resemble usage"](https://testing-library.com/docs/solid-testing-library/intro/) — the guiding principle for what to assert (DOM + user-facing semantics, not implementation details).
- `userEvent.setup()` recommended over the older fire-event style — closer to real user input timing.

---

## 8. Acceptance criteria

1. `npm test` runs the new integration suite alongside existing Vitest unit tests; both pass.
2. The §4.1 regression test fails on Stage 0 code (before the lift) and passes on `main` (Stage 1+ shipped).
3. CI green; no new infrastructure beyond `package.json` dev deps + a tiny `vitest-setup.ts`.
4. Suite runs in under 10 seconds locally.
