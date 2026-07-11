// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * E2E regression test for the `Client.windowids` window-close baseline
 * (task #29): every open+close cycle of a secondary window must return
 * srv's durable window list to its pre-open state — through BOTH close
 * entry points, since they route differently in the host:
 *
 *   - `close_window` (custom chrome ✕ button, Cmd+W) — the path Round 6
 *     of retro-window-lifecycle-leak-2026-07-04 made clean;
 *   - `close_window_by_label` (tear-off merge path) — which posted a raw
 *     WM_CLOSE that CEF 148's Views wndproc turns into a parked browser
 *     with NO srv cleanup, leaking the window's srv rows and windowids
 *     entry forever (fixed alongside this test).
 *
 * `Client.windowids` is the slow-path crash-reproject's source of truth
 * (SPEC_PILLAR1_STEP4): every leaked entry here becomes a phantom window
 * resurrected on some future crash recovery, which is how this leak was
 * originally discovered.
 *
 * Same opt-in gate as `crash-reproject.e2e.test.ts` (a real packaged
 * build is required):
 *
 *   AGENTMUX_E2E_BUILD_DIR="/c/Users/you/Desktop/agentmux-...-x64-portable" \
 *     npx vitest run test/e2e/window-close-baseline.e2e.test.ts
 */

import { describe, it, expect, beforeAll, afterAll } from "vitest";
import {
    bootInstance,
    teardownInstance,
    evalInMainWindow,
    findCdpPort,
    sleep,
    type E2eInstance,
} from "./harness";

const BUILD_DIR = process.env.AGENTMUX_E2E_BUILD_DIR;
const IS_WINDOWS = process.platform === "win32";
const RUN = Boolean(BUILD_DIR) && IS_WINDOWS;

if (!RUN) {
    const reason = !BUILD_DIR
        ? "AGENTMUX_E2E_BUILD_DIR not set"
        : `unsupported platform (${process.platform}, Windows-only)`;
    // eslint-disable-next-line no-console
    console.log(`[window-close-baseline e2e] skipped — ${reason}`);
}

/** Fetch srv's `Client.windowids` via the same `client.GetClientData` RPC
 *  the host's own slow-path reproject reads, driven from the main window's
 *  page context (which holds the web endpoint + auth key). */
function windowIdsExpr(): string {
    return `(async () => {
        const ep = window.__WAVE_SERVER_WEB_ENDPOINT__;
        const key = await window.api.getAuthKey();
        const res = await fetch("http://" + ep + "/agentmux/service", {
            method: "POST",
            headers: { "Content-Type": "application/json", "X-AuthKey": key },
            body: JSON.stringify({ service: "client", method: "GetClientData", args: [], uicontext: null }),
        });
        const j = await res.json();
        return (j && j.data && j.data.windowids) || j.windowids || [];
    })()`;
}

/** Poll until `Client.windowids` has exactly `expected` entries (the srv
 *  cleanup runs on a background thread with a bounded registration-race
 *  retry — see `demote_srv_cleanup` — so a fixed sleep would be flaky). */
async function pollWindowIdsCount(port: number, expected: number, timeoutMs: number): Promise<string[]> {
    const deadline = Date.now() + timeoutMs;
    let ids: string[] = [];
    while (Date.now() < deadline) {
        ids = await evalInMainWindow(port, windowIdsExpr());
        if (ids.length === expected) return ids;
        await sleep(1000);
    }
    return ids;
}

/** Open one window, close it via `closeExpr`, and assert `Client.windowids`
 *  returns to its pre-open state.
 *
 *  All assertions are RELATIVE to the observed pre-open baseline, never an
 *  absolute count: suites in this directory share the build's per-build
 *  data dir (`--no-file-parallelism` serializes them, but srv state is
 *  durable across boots BY DESIGN — it's the crash-reproject slow path's
 *  source of truth), and the crash-reproject suite deliberately leaves the
 *  window it recreated behind. An absolute `=== 1` here would couple this
 *  suite to file ordering; open→N+1, close→N is the actual invariant. */
async function runOpenCloseCycle(port: number, closeExpr: (label: string) => string): Promise<void> {
    const baseline: string[] = await evalInMainWindow(port, windowIdsExpr());
    expect(baseline.length).toBeGreaterThanOrEqual(1); // at least main is registered

    const newLabel = await evalInMainWindow(port, "window.api.openNewWindow()");
    expect(typeof newLabel).toBe("string");

    // The new window's own bootstrap registers one more srv window row.
    const afterOpen = await pollWindowIdsCount(port, baseline.length + 1, 20000);
    expect(afterOpen.length).toBe(baseline.length + 1);

    await evalInMainWindow(port, closeExpr(newLabel));

    const afterClose = await pollWindowIdsCount(port, baseline.length, 20000);
    expect(afterClose.length).toBe(baseline.length);
    // Same SET of rows as before the open — the new row is gone and no
    // pre-existing row (e.g. main's) was collateral damage.
    expect([...afterClose].sort()).toEqual([...baseline].sort());
}

describe.skipIf(!RUN)("window-close baseline: Client.windowids returns to baseline", () => {
    let inst: E2eInstance | null = null;
    let port = 0;

    beforeAll(async () => {
        inst = await bootInstance(BUILD_DIR!, "window-close-baseline-e2e.log");
        port = await findCdpPort(inst.hostPid!, 15000);
        // Let main's own bootstrap (workspace init + register_backend_window)
        // land before measuring any baseline — and, if a prior suite left
        // durable window rows behind, let the slow-path reproject finish
        // recreating them so the count is stable before the first test reads
        // it. "Stable" = two consecutive reads 3s apart agree.
        const deadline = Date.now() + 30000;
        let prev = -1;
        while (Date.now() < deadline) {
            const ids: string[] = await evalInMainWindow(port, windowIdsExpr());
            if (ids.length >= 1 && ids.length === prev) break;
            prev = ids.length;
            await sleep(3000);
        }
    }, 60000);

    afterAll(async () => {
        if (inst) await teardownInstance(BUILD_DIR!, inst);
    });

    it("open + closeWindow (chrome ✕ path) returns windowids to baseline", async () => {
        await runOpenCloseCycle(port, (label) => `window.api.closeWindow(${JSON.stringify(label)})`);
    }, 90000);

    it("open + closeWindowByLabel (tear-off merge path) returns windowids to baseline", async () => {
        // Regression for task #29 round 2: this exact sequence used to leave
        // the promoted window's srv row orphaned forever (raw WM_CLOSE →
        // parked browser → demote_srv_cleanup never ran).
        await runOpenCloseCycle(port, (label) => `window.api.closeWindowByLabel(${JSON.stringify(label)})`);
    }, 90000);
});
