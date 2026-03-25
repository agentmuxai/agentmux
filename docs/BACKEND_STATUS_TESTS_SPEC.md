# Test Spec: Backend Status Atom (`backendStatusAtom`)

**Date:** 2026-03-25
**File to create:** `frontend/app/store/global.test.ts`
**Runner:** Vitest (existing setup in `vitest.config.ts`)
**Pattern:** mirrors `frontend/app/view/agent/state.test.ts` — SolidJS signals tested directly, no JSDOM required for pure signal logic

---

## What we're testing

`backendStatusAtom` has three states: `"connecting" | "running" | "crashed"`.

There are exactly **four transitions** that matter:

| From | To | Trigger |
|------|----|---------|
| `"connecting"` | `"running"` | `backend-ready` event fires (normal startup) |
| `"connecting"` | `"running"` | backend already up when listener registers (catch-up check) |
| `"running"` | `"crashed"` | `backend-terminated` event fires |
| `"crashed"` | `"connecting"` | user clicks Restart (handleRestart sets atom directly) |
| `"connecting"` | `"connecting"` | `backend-ready` fires but `getBackendInfo` fails (atom unchanged — should not flip to running) |

The bug we just fixed (stuck `"connecting"`) came from the missing catch-up check. Tests would have caught it.

---

## Mocking strategy

`global.ts` is heavily coupled to `getApi()` (which reads `window.api`) and several `@/store/*` imports. The test file should **not** import `global.ts` directly — that pulls in SolidJS reactive roots, layout imports, RpcApi, and more.

Instead, extract the **pure state logic** into a testable unit by testing the signal layer in isolation.

### Approach A — Test the signals directly (preferred)

The signals (`backendStatusAtom`, `setBackendStatusAtom`, `backendDeathInfoAtom`, `setBackendDeathInfoAtom`) can be imported directly. The event-listener wiring in `initGlobalSignals` can be exercised by calling the listener callbacks manually with mock payloads — no Tauri IPC needed.

```typescript
// Pattern:
import { createRoot } from "solid-js";
import {
    backendStatusAtom,
    setBackendStatusAtom,
    backendDeathInfoAtom,
    setBackendDeathInfoAtom,
} from "@/store/global";
```

The atom reads run inside `createRoot` to suppress SolidJS "no reactive owner" warnings:

```typescript
function read<T>(signal: () => T): T {
    let val!: T;
    createRoot((dispose) => { val = signal(); dispose(); });
    return val;
}
```

### Approach B — Extract a `backendStatus.ts` module (cleaner long-term)

Move the backend status signals + listener setup into their own file:

```
frontend/app/store/backendStatus.ts
```

This file would own:
- `BackendDeathInfo` interface
- `backendStatusAtom` / `backendDeathInfoAtom` signals
- `initBackendStatusListeners(api, onReconnect)` function

Then `global.ts` calls `initBackendStatusListeners(getApi(), reconnectWS)`. The test imports `backendStatus.ts` directly — no giant global.ts dep tree.

**Recommendation:** Start with Approach A for this PR (zero refactor, just tests). Switch to Approach B when `global.ts` is modularized.

---

## Test cases

### Suite 1: Initial state

```typescript
describe("backendStatusAtom — initial state", () => {
    test("starts as connecting", () => {
        expect(read(backendStatusAtom)).toBe("connecting");
    });

    test("backendDeathInfoAtom starts null", () => {
        expect(read(backendDeathInfoAtom)).toBeNull();
    });
});
```

---

### Suite 2: backend-ready event → running

```typescript
describe("backendStatusAtom — backend-ready", () => {
    beforeEach(() => setBackendStatusAtom("connecting"));

    test("transitions to running when backend-ready fires", () => {
        // Simulate what the initGlobalSignals backend-ready listener does
        setBackendStatusAtom("running");
        expect(read(backendStatusAtom)).toBe("running");
    });

    test("stays running if backend-ready fires twice", () => {
        setBackendStatusAtom("running");
        setBackendStatusAtom("running");
        expect(read(backendStatusAtom)).toBe("running");
    });
});
```

---

### Suite 3: catch-up check (the bug we fixed)

This is the core regression test. Simulates the scenario where `backend-ready` fires before the listener is registered, and the catch-up `getBackendInfo` call resolves it.

```typescript
describe("backendStatusAtom — catch-up when backend already up", () => {
    beforeEach(() => setBackendStatusAtom("connecting"));

    test("transitions to running when getBackendInfo succeeds and status is connecting", async () => {
        // Simulate the catch-up logic in initGlobalSignals:
        //   getApi().getBackendInfo().then(() => {
        //       if (backendStatusAtom() === "connecting") setBackendStatusAtom("running");
        //   })
        const mockGetBackendInfo = vi.fn().mockResolvedValue({ version: "0.32.81" });
        await mockGetBackendInfo();
        if (read(backendStatusAtom) === "connecting") {
            setBackendStatusAtom("running");
        }
        expect(read(backendStatusAtom)).toBe("running");
    });

    test("does NOT transition to running if already crashed", async () => {
        setBackendStatusAtom("crashed");
        const mockGetBackendInfo = vi.fn().mockResolvedValue({ version: "0.32.81" });
        await mockGetBackendInfo();
        // catch-up only applies when "connecting", not "crashed"
        if (read(backendStatusAtom) === "connecting") {
            setBackendStatusAtom("running");
        }
        expect(read(backendStatusAtom)).toBe("crashed");
    });

    test("does NOT transition to running if getBackendInfo fails", async () => {
        const mockGetBackendInfo = vi.fn().mockRejectedValue(new Error("not ready"));
        await mockGetBackendInfo().catch(() => {});
        // on failure, atom stays connecting (backend not yet up)
        expect(read(backendStatusAtom)).toBe("connecting");
    });
});
```

---

### Suite 4: backend-terminated → crashed

```typescript
describe("backendStatusAtom — backend-terminated", () => {
    beforeEach(() => setBackendStatusAtom("running"));

    test("transitions to crashed on termination", () => {
        const deathPayload = {
            code: -1073740791,
            signal: null,
            pid: 12345,
            uptime_secs: 9173,
            died_at: new Date().toISOString(),
        };
        setBackendDeathInfoAtom(deathPayload);
        setBackendStatusAtom("crashed");

        expect(read(backendStatusAtom)).toBe("crashed");
        expect(read(backendDeathInfoAtom)).toMatchObject({
            code: -1073740791,
            pid: 12345,
            uptime_secs: 9173,
        });
    });

    test("preserves death info after crashing", () => {
        setBackendDeathInfoAtom({ code: 1, signal: null, pid: 999, uptime_secs: 35, died_at: "2026-03-24T23:36:59Z" });
        setBackendStatusAtom("crashed");
        expect(read(backendDeathInfoAtom)?.code).toBe(1);
        expect(read(backendDeathInfoAtom)?.uptime_secs).toBe(35);
    });

    test("exit code null when signal kill", () => {
        setBackendDeathInfoAtom({ code: null, signal: 9, pid: 999, uptime_secs: 100, died_at: new Date().toISOString() });
        setBackendStatusAtom("crashed");
        expect(read(backendDeathInfoAtom)?.code).toBeNull();
        expect(read(backendDeathInfoAtom)?.signal).toBe(9);
    });
});
```

---

### Suite 5: restart cycle

```typescript
describe("backendStatusAtom — restart cycle", () => {
    test("full cycle: connecting → running → crashed → connecting → running", () => {
        // startup
        expect(read(backendStatusAtom)).toBe("connecting");
        setBackendStatusAtom("running");
        expect(read(backendStatusAtom)).toBe("running");

        // crash
        setBackendDeathInfoAtom({ code: -1073740791, signal: null, pid: 1, uptime_secs: 3600, died_at: new Date().toISOString() });
        setBackendStatusAtom("crashed");
        expect(read(backendStatusAtom)).toBe("crashed");

        // user clicks Restart
        setBackendStatusAtom("connecting");
        expect(read(backendStatusAtom)).toBe("connecting");
        expect(read(backendDeathInfoAtom)).not.toBeNull(); // death info preserved until backend-ready

        // restart succeeds
        setBackendStatusAtom("running");
        expect(read(backendStatusAtom)).toBe("running");
    });

    test("death info is preserved through connecting state after restart", () => {
        const deathInfo = { code: -1073740791, signal: null, pid: 1, uptime_secs: 3600, died_at: "t" };
        setBackendDeathInfoAtom(deathInfo);
        setBackendStatusAtom("crashed");
        setBackendStatusAtom("connecting"); // restart initiated
        expect(read(backendDeathInfoAtom)).not.toBeNull(); // still visible in "was up X" line
    });
});
```

---

## File structure

```
frontend/app/store/
  global.ts
  global.test.ts         ← new
```

`global.test.ts` header:

```typescript
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, test, expect, beforeEach, vi } from "vitest";
import { createRoot } from "solid-js";
import {
    backendStatusAtom,
    setBackendStatusAtom,
    backendDeathInfoAtom,
    setBackendDeathInfoAtom,
} from "./global";

/** Read a SolidJS signal outside a reactive root without warnings. */
function read<T>(signal: () => T): T {
    let val!: T;
    createRoot((dispose) => { val = signal(); dispose(); });
    return val;
}
```

---

## Import hazard

`global.ts` imports from `@/layout/index`, `@/app/store/wshrpcutil`, and other heavy modules. Vitest may fail to resolve these in the test environment.

**Mitigations (in order of preference):**
1. Mock the problematic imports in `vitest.config.ts` using `alias` or inline `vi.mock()`
2. Extract `backendStatus.ts` (Approach B above) to isolate the signals from the heavy deps
3. Use `vi.mock("@/app/store/wshrpcutil", () => ({}))` at the top of the test file

The `state.test.ts` pattern (testing signals directly without mocking layout deps) works because `state.ts` doesn't import from layout. `global.ts` does — so some mock setup will be needed. Document the required mocks in the test file header when implementing.
