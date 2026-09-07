// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pins A9 of the architecture-refactor program (issue #1549,
 * `docs/analysis/TRACKING_ARCHITECTURE_REFACTOR_A1_A15_2026_06_18.md`):
 * `agent-view.tsx` must dispatch against the agent-pane stores ONLY through
 * its `AgentPaneModel` handle, never via the module-level `dispatch` /
 * `dispatchIfRegistered` exports.
 *
 * Why it matters: the model is the one place that (a) drops dispatches after
 * `unregisterPane` instead of throwing — the hard `dispatch` throws on an
 * unregistered pane, which is exactly the post-unmount race a view-level
 * async callback hits — and (b) writes the `agent:dispatchPane` render-trail
 * point the crash boundary dumps. A raw call bypasses both. The audit found
 * 11 such bypasses in this file; routing them was the fix.
 *
 * Grep-shaped, same as `flows/run-cli-login-single-caller.test.ts`: a future
 * direct import of the store dispatchers into this file fails a test, not a
 * user's pane.
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const AGENT_VIEW = join(__dirname, "agent-view.tsx");

describe("agent-view.tsx dispatches only through its AgentPaneModel (A9)", () => {
    const text = readFileSync(AGENT_VIEW, "utf8");

    it("does not import the store-level dispatch helpers", () => {
        // Both stores export `dispatch` and `dispatchIfRegistered`; agent-view
        // used to alias them as dispatchPane / dispatchPaneIfRegistered /
        // dispatchDocIfRegistered. Any of those names appearing in an import
        // means the bypass is back.
        const importBlocks = text.match(/^import[\s\S]*?from\s+"[^"]+";/gm) ?? [];
        const offending = importBlocks.filter((block) =>
            /\bdispatch(?:IfRegistered)?\b(?!Doc\b)/.test(block) &&
            /agent-(?:pane-state|document)-store/.test(block),
        );
        expect(offending).toEqual([]);
    });

    it("has no raw dispatch* call sites — every dispatch goes via paneModel.", () => {
        // A raw call is `dispatchPane(` / `dispatchDoc(` / the *IfRegistered
        // forms NOT preceded by `paneModel.` (or any other member access).
        const raw = text.match(/(?<![.\w])dispatch(?:Pane|Doc)(?:IfRegistered)?\(/g) ?? [];
        expect(raw).toEqual([]);
    });

    it("still dispatches — the routing did not silently delete call sites", () => {
        // Guard against the trivial way to satisfy the test above. The audit
        // counted 11; the number may legitimately move, but it must not be 0.
        const routed = text.match(/\bpaneModel\.dispatch(?:Pane|Doc)\(/g) ?? [];
        expect(routed.length).toBeGreaterThan(0);
    });
});
