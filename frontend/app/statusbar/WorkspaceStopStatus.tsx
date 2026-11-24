// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { t } from "@/app/i18n/i18n";
import { clockTime, countdown, startWorkspaceStop, workspaceStop } from "@/app/store/workspace-stop";
import { createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";

/**
 * A hosted workspace is going to stop: how long is left and why, so nobody is surprised by it
 * (the workspaces-in-the-Console spec, sections 6.3 and 6.4). Only AgentMux Web has a host to tell
 * srv, so a desktop never shows it. The data is saved when it stops, and the workspace is started again from the Console. Keep
 * running and Stop now come with the workspace's own routes; this says what is going to happen.
 */
const WorkspaceStopStatus = (): JSX.Element => {
    onMount(startWorkspaceStop);
    const [now, setNow] = createSignal(Date.now());
    onMount(() => {
        const timer = setInterval(() => setNow(Date.now()), 1000);
        onCleanup(() => clearInterval(timer));
    });
    return (
        <Show when={workspaceStop()}>
            {(stop) => {
                const tip = () => t("workspaceStop.tip", { time: clockTime(stop().stopAtMs), reason: stop().reason });
                return (
                    <div class="status-bar-item" role="status" data-tip={tip()} aria-label={tip()}>
                        <i class="fa-solid fa-power-off status-icon" style={{ color: "var(--warning-text-color)" }} />
                        <span style={{ color: "var(--warning-text-color)" }}>{t("workspaceStop.chip", { left: countdown(stop().stopAtMs, now()) })}</span>
                    </div>
                );
            }}
        </Show>
    );
};

WorkspaceStopStatus.displayName = "WorkspaceStopStatus";

export { WorkspaceStopStatus };
