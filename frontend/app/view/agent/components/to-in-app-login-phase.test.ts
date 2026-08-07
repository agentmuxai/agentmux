// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for `toInAppLoginPhase` — the mapping that drives the credential-loss
 * relogin surface's InAppLoginPanel phase line (Fix 3b of
 * ANALYSIS_ARMORY_STASH_CREDENTIAL_VISIBILITY_GAP_2026_08_04.md). Derived
 * from useAgentControllerStatus.ts's existing `authUrl`/`launchPhase`
 * signals without changing that hook — this pins the derivation itself,
 * particularly the one non-obvious case: `LaunchPhase`'s
 * "waiting-for-login-completion" variant covers BOTH tier 1's post-URL wait
 * and tier 3's terminal-completion poll (see launch-phase.ts's own doc
 * comment on that variant), and `authUrl` presence is what disambiguates
 * them here.
 */

import { describe, expect, it } from "vitest";
import { toInAppLoginPhase } from "./to-in-app-login-phase";
import type { LaunchPhase } from "../flows/launch-phase";

describe("toInAppLoginPhase", () => {
    it("returns 'starting' when there is no launch phase yet", () => {
        expect(toInAppLoginPhase(null, null)).toBe("starting");
    });

    it("returns 'starting' for phases before a URL/terminal decision is reached", () => {
        const phases: LaunchPhase[] = [
            { kind: "resolving-cli" },
            { kind: "checking-auth" },
            { kind: "waiting-for-login-link", deadlineMs: 0 },
        ];
        for (const phase of phases) {
            expect(toInAppLoginPhase(null, phase)).toBe("starting");
        }
    });

    it("returns 'fallback' once the terminal is being opened, regardless of a stale authUrl", () => {
        const phase: LaunchPhase = { kind: "opening-login-terminal" };
        expect(toInAppLoginPhase(null, phase)).toBe("fallback");
        expect(toInAppLoginPhase("https://example.com/authorize", phase)).toBe("fallback");
    });

    it("returns 'waiting-authorize' during the completion wait WHEN a URL was shown (the in-app path)", () => {
        const phase: LaunchPhase = { kind: "waiting-for-login-completion", deadlineMs: 0 };
        expect(toInAppLoginPhase("https://example.com/authorize", phase)).toBe("waiting-authorize");
    });

    it("returns 'terminal-polling' during the completion wait when NO url was ever shown (the terminal path)", () => {
        const phase: LaunchPhase = { kind: "waiting-for-login-completion", deadlineMs: 0 };
        expect(toInAppLoginPhase(null, phase)).toBe("terminal-polling");
    });

    it("returns 'starting' for post-completion phases (verifying/ready/failed) — nothing left to show", () => {
        const phases: LaunchPhase[] = [
            { kind: "verifying" },
            { kind: "fresh-ready" },
            { kind: "resumed-ready" },
            { kind: "failed", reason: "boom" },
        ];
        for (const phase of phases) {
            expect(toInAppLoginPhase(null, phase)).toBe("starting");
        }
    });
});
