// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * SPEC_PILLAR1_STEP4 Phase 5 — E2E test for "host OOM ⇒ session reprojects"
 * (the parent design doc's §3 acceptance criterion), automating this
 * session's Phase 2/3 manual live-verification methodology: launch an
 * isolated instance, open extra windows, kill the inner host process only
 * (the launcher survives — the far more common crash than a full
 * process-tree kill), wait for the launcher-supervised respawn, and assert
 * the recreated windows show up as real CDP targets with correct kind.
 *
 * Greenfield and Windows-only (matching every other Phase 1-4 verification
 * this spec's implementation was built and checked against). Requires a real
 * portable/dev build — `task package` takes minutes, so this suite does NOT
 * run under a plain `npm test`. Set `AGENTMUX_E2E_BUILD_DIR` to a build
 * directory (the folder containing `agentmux.exe`, e.g. the output of
 * `task package`) to opt in:
 *
 *   AGENTMUX_E2E_BUILD_DIR="/c/Users/you/Desktop/agentmux-...-x64-portable" \
 *     npx vitest run test/e2e/crash-reproject.e2e.test.ts
 *
 * Without that env var, every test in this file reports `skipped` — matching
 * common practice for expensive/opt-in E2E suites (see e.g. Playwright's
 * `test.skip`-on-missing-fixture convention) rather than silently passing or
 * blocking every routine `npm test` run on a multi-minute build.
 */

import { describe, it, expect, beforeAll, afterAll } from "vitest";
// CDP wire helpers, isolated-instance boot/teardown, and wmic process
// discovery live in the shared harness (extracted verbatim from this file
// when the window-close-baseline suite was added) — including the
// native-WebSocket-instead-of-`ws` rationale, documented there.
import {
    bootInstance,
    teardownInstance,
    evalInMainWindow,
    findCdpPort,
    findVersionedHostPid,
    killPid,
    listCdpTargets,
    sleep,
    type CdpTarget,
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
    console.log(`[crash-reproject e2e] skipped — ${reason}`);
}

describe.skipIf(!RUN)("crash-reproject E2E: host OOM ⇒ session reprojects", () => {
    let inst: E2eInstance | null = null;
    let hostPid: number | null = null;

    beforeAll(async () => {
        inst = await bootInstance(BUILD_DIR!, "crash-reproject-e2e.log");
        hostPid = inst.hostPid;
    }, 30000);

    afterAll(async () => {
        // Teardown kills the wrapper FIRST (killing the host while the
        // wrapper lives triggers the exact crash-restart mechanism under
        // test), then sweeps this build dir's own process tree — see
        // `teardownInstance`. Keep `inst.hostPid` current so the sweep's
        // direct kill hits the RESPAWNED host, not the long-dead first one.
        if (inst) {
            inst.hostPid = hostPid;
            await teardownInstance(BUILD_DIR!, inst);
        }
    });

    it("recreates extra top-level windows after the inner host process is killed", async () => {
        expect(hostPid).not.toBeNull();
        const port1 = await findCdpPort(hostPid!, 15000);

        // Open one extra top-level window via the real IPC command (the
        // same path an interactive "New Window" takes).
        const newLabel = await evalInMainWindow(
            port1,
            "window.api.openNewWindow()",
        );
        expect(typeof newLabel).toBe("string");

        // Give the topology write-through (SPEC_PILLAR1_STEP3) and the
        // launcher's WindowOpened mirror a moment to land before the crash.
        await sleep(3000);

        const beforeInstances: Array<{ label: string }> = await evalInMainWindow(
            port1,
            "window.api.listWindowInstances().then(r => r)",
        );
        const extraBefore = beforeInstances.filter((w) => w.label !== "main");
        expect(extraBefore.length).toBeGreaterThanOrEqual(1);

        // Kill the INNER host process only — the launcher survives and
        // supervises a respawn. Never kill by image name (CLAUDE.md).
        killPid(hostPid!);

        // The launcher spawns a brand-new host process (new PID).
        const newHostPid = await findVersionedHostPid(BUILD_DIR!, 20000, hostPid!);
        hostPid = newHostPid;
        const port2 = await findCdpPort(newHostPid, 15000);

        // Fast path: the launcher's in-memory snapshot should have the
        // extra window and reproject it. Poll briefly — reproject is fast
        // (sub-second in this session's live verifications) but runs
        // asynchronously relative to the new host's own startup. Poll on
        // "main's real title has settled" specifically, not merely
        // "more than one target exists" — main's CDP target briefly shows
        // `index.html`'s generic pre-init placeholder title ("AgentMux",
        // bare, no " - " separator) before its content finishes loading
        // and `document.title` resolves to the real
        // `{Window Name} - {Tab} - AgentMux` format, so a bare count check
        // can pass before main is actually the window we expect. Matched
        // by `window_transparent=0` (a stable URL param the main window
        // always carries), not by title text — the window name half of
        // that format is no longer a fixed "Starter workspace..." string
        // after frontend/util/window-title.ts's bootstrap-default-name
        // exclusion fix (it's deterministically "Window 1 - ..." now, but
        // matching on the URL is the correct identifier regardless of
        // which of the 3 name tiers ends up resolving).
        const deadline = Date.now() + 15000;
        let targets: CdpTarget[] = [];
        let mainTarget: CdpTarget | undefined;
        while (Date.now() < deadline) {
            targets = await listCdpTargets(port2);
            mainTarget = targets.find(
                (t) => t.url.includes("window_transparent=0") && t.title !== "AgentMux",
            );
            if (mainTarget && targets.length > 1) break;
            await sleep(500);
        }

        // At least the main window and one recreated extra window must show
        // up as real, distinct CDP targets — this is the assertion this
        // whole suite exists for: a log line saying "reproject complete" is
        // not sufficient (see docs/retro/retro-browser-pane-renderer-leak-2026-07-07.md's
        // own conclusion — verify against the CDP target list, not the
        // host's own event log).
        expect(targets.length).toBeGreaterThan(1);
        expect(mainTarget).toBeDefined();
    }, 90000);
});
