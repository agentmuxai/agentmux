// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { getApi, windowCountAtom, backendStatusAtom, isDev } from "@/store/global";
import { createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";
import { InstancePanel } from "./InstancePanel";

/** The version chip at the right end of the status bar, and the instance
 *  panel it opens. */
export const VersionChip = (): JSX.Element => {
    const version = getApi().getAboutModalDetails()?.version ?? "";
    const windowCount = windowCountAtom;

    // The chip is the button, or the offline span while the backend is down.
    // The instance panel anchors to whichever is showing ("Open Maintenance"
    // can open it while the backend is down).
    let versionRef: HTMLElement | undefined;
    const [panelOpen, setPanelOpen] = createSignal(false);

    // BackendStatus dot can request the version panel to open (e.g. "Open Maintenance ↗").
    onMount(() => {
        const handler = () => setPanelOpen(true);
        window.addEventListener("agentmux:open-version-panel", handler);
        onCleanup(() => window.removeEventListener("agentmux:open-version-panel", handler));
    });

    return (
        <>
            <Show when={version}>
                <Show
                    when={backendStatusAtom() !== "crashed"}
                    fallback={
                        <span
                            ref={(el) => { versionRef = el; }}
                            class="status-version status-version-offline"
                            data-tip="Backend offline"
                            aria-label="Backend offline"
                        >
                            v{version}
                            <Show when={isDev()}>
                                <span class="status-version-dev">DEV</span>
                            </Show>
                        </span>
                    }
                >
                    <button
                        ref={(el) => { versionRef = el; }}
                        type="button"
                        class="status-version clickable"
                        onClick={() => setPanelOpen(!panelOpen())}
                        data-tip="Click for instance panel"
                        aria-label="AgentMux version — open instance panel"
                        aria-haspopup="dialog"
                        aria-expanded={panelOpen()}
                    >
                        v{version}
                        <Show when={isDev()}>
                            <span class="status-version-dev">DEV</span>
                        </Show>
                        <Show when={windowCount() > 1}>
                            <span class="instance-num"> ({windowCount()})</span>
                        </Show>
                    </button>
                </Show>
            </Show>
            <Show when={panelOpen()}>
                <InstancePanel anchor={versionRef} onClose={() => setPanelOpen(false)} />
            </Show>
        </>
    );
};
