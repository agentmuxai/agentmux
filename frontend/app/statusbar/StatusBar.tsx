// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { getApi, windowInstanceNumAtom, windowCountAtom, backendStatusAtom } from "@/store/global";
import { createEffect, createSignal, onCleanup, Show, type JSX } from "solid-js";
import { BackendStatus } from "./BackendStatus";
import { ConfigStatus } from "./ConfigStatus";
import { HostPopover } from "./HostPopover";
import { InstancePanel } from "./InstancePanel";
import { SystemStats } from "./SystemStats";
import { TokenUsageIndicator } from "./TokenUsageIndicator";
import { UpdateStatus } from "./UpdateStatus";
import "./StatusBar.scss";

const StatusBar = (): JSX.Element => {
    const version = getApi().getAboutModalDetails()?.version ?? "";
    const instanceNum = windowInstanceNumAtom;
    const windowCount = windowCountAtom;

    let versionRef!: HTMLButtonElement;
    const [panelOpen, setPanelOpen] = createSignal(false);
    const [anchorRect, setAnchorRect] = createSignal<DOMRect | null>(null);

    const handleVersionClick = () => {
        if (panelOpen()) {
            setPanelOpen(false);
            return;
        }
        setAnchorRect(versionRef?.getBoundingClientRect() ?? null);
        setPanelOpen(true);
    };

    // Esc + click-outside close. Mirrors TokenBreakdownPopover precedent.
    createEffect(() => {
        if (!panelOpen()) return;
        const onKey = (e: KeyboardEvent) => {
            if (e.key === "Escape") setPanelOpen(false);
        };
        const onClick = (e: MouseEvent) => {
            const t = e.target as Node;
            if (versionRef?.contains(t)) return;
            const panelEl = document.querySelector(".instance-panel");
            if (panelEl?.contains(t)) return;
            setPanelOpen(false);
        };
        document.addEventListener("keydown", onKey);
        document.addEventListener("mousedown", onClick);
        onCleanup(() => {
            document.removeEventListener("keydown", onKey);
            document.removeEventListener("mousedown", onClick);
        });
    });

    return (
        <div class="status-bar">
            <div class="status-bar-left">
                <BackendStatus />
                <span class="stat-separator">|</span>
                <SystemStats />
            </div>
            <div class="status-bar-center" />
            <div class="status-bar-right">
                <TokenUsageIndicator />
                <ConfigStatus />
                <UpdateStatus />
                <HostPopover />
                <Show when={version}>
                    <Show
                        when={backendStatusAtom() !== "crashed"}
                        fallback={
                            <span
                                class="status-version status-version-offline"
                                data-tip="Backend offline"
                                aria-label="Backend offline"
                            >
                                v{version}
                            </span>
                        }
                    >
                        <button
                            ref={versionRef!}
                            type="button"
                            class="status-version clickable"
                            onClick={handleVersionClick}
                            data-tip="Click for instance panel"
                            aria-label="AgentMux version — open instance panel"
                            aria-haspopup="dialog"
                            aria-expanded={panelOpen()}
                        >
                            v{version}
                            <Show when={windowCount() > 1}>
                                <span class="instance-num"> ({instanceNum()})</span>
                            </Show>
                        </button>
                    </Show>
                </Show>
            </div>
            <Show when={panelOpen()}>
                <InstancePanel anchorRect={anchorRect()} onClose={() => setPanelOpen(false)} />
            </Show>
        </div>
    );
};

StatusBar.displayName = "StatusBar";

export { StatusBar };
