// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { LaunchPhase } from "../flows/launch-phase";
import type { InAppLoginPhase } from "./InAppLoginPanel";

/**
 * Derive InAppLoginPanel's narrower phase enum from
 * useAgentControllerStatus.ts's existing `LaunchPhase` + `authUrl` signals
 * — no changes to that hook's own (already extensively hardened) phase
 * tracking. `waiting-for-login-completion` covers BOTH tier 1's post-URL
 * wait and tier 3's terminal-completion poll (see that variant's own doc
 * comment in launch-phase.ts); `authUrl` presence is what actually
 * distinguishes them here — a URL was shown (in-app path) vs. none ever
 * was (the terminal was opened instead).
 */
export function toInAppLoginPhase(authUrl: string | null, launchPhase: LaunchPhase | null): InAppLoginPhase {
    if (launchPhase?.kind === "opening-login-terminal") return "fallback";
    if (launchPhase?.kind === "waiting-for-login-completion") {
        return authUrl ? "waiting-authorize" : "terminal-polling";
    }
    return "starting";
}
