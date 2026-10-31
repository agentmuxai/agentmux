// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { browserPauseAll, PAUSED_BY_YOU, setAgentBrowsingPaused, startBrowserPauseAll } from "@/app/store/browser-pause-all";
import { onMount, Show, type JSX } from "solid-js";

/**
 * "Pause all agent browsing" is on: every pane an agent drives refuses its
 * calls until you resume. It shows the reason, so nobody takes it for a fault,
 * and one click on it resumes.
 */
const BrowserPauseAllStatus = (): JSX.Element => {
    onMount(startBrowserPauseAll);
    const tip = () => `${PAUSED_BY_YOU}: every browser pane an agent drives refuses its calls. Click to resume`;
    return (
        <Show when={browserPauseAll().paused}>
            <div
                class="status-bar-item clickable"
                role="button"
                data-tip={tip()}
                aria-label={tip()}
                onClick={() => void setAgentBrowsingPaused(false).catch((e) => console.warn("[browser-pause-all] couldn't resume", e))}
            >
                <i class="fa-solid fa-pause status-icon" style={{ color: "var(--warning-text-color)" }} />
                <span style={{ color: "var(--warning-text-color)" }}>{PAUSED_BY_YOU}. Resume</span>
            </div>
        </Show>
    );
};

BrowserPauseAllStatus.displayName = "BrowserPauseAllStatus";

export { BrowserPauseAllStatus };
