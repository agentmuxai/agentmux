// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * TabModalLayer — per-tab modal host.
 *
 * Provides a context for opening tab-scoped modals and renders the
 * overlay as a sibling of `<TileLayout>` inside `<TabContent>`. No
 * extra DOM wrappers are added around the tile layout — `<Context.Provider>`
 * emits no DOM node, so the existing flex layout in TabContent is
 * completely undisturbed. The overlay uses `position:absolute; inset:0`
 * against TabContent's `position:relative` root div.
 *
 * Switching tabs hides the modal via the existing `display:none` on
 * inactive tab content. The top tab bar stays interactive.
 *
 * The legacy global `Modal` (modal-v2) stays for window-level dialogs
 * (command palette, about, backend prompts). This layer is additive.
 *
 * See docs/specs/launch-modal-rearchitecture-2026-05-01.md.
 */

import { createSignal, Show, type Component, type JSX } from "solid-js";

import { AgentLaunchModalPanel } from "@/app/view/agent/components/AgentLaunchModal";

import { TabModalContext, type TabModalApi, type TabModalRequest } from "./tab-modal";
import "./tab-modal.scss";

interface TabModalLayerProps {
    children: JSX.Element;
}

export const TabModalLayer: Component<TabModalLayerProps> = (props) => {
    const [current, setCurrent] = createSignal<TabModalRequest | null>(null);

    const api: TabModalApi = {
        open: (req) => setCurrent(req),
        close: () => setCurrent(null),
        current,
    };

    // Backdrop click closes. Panel click is stopped from bubbling so a
    // click inside the form doesn't reach the backdrop. ESC is handled
    // on the overlay itself so we don't need a wrapper div around
    // TileLayout (which would break its flex layout).
    const handleBackdropClick = (e: MouseEvent) => {
        if (e.target === e.currentTarget) {
            api.close();
        }
    };

    const handleOverlayKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Escape") {
            e.stopPropagation();
            api.close();
        }
    };

    // Context.Provider emits no DOM node, so TileLayout (props.children)
    // remains a direct child of TabContent's flex container. The overlay
    // is a sibling rendered after it; `position:absolute; inset:0` on
    // .tab-modal-overlay pins it to TabContent's `position:relative` root.
    return (
        <TabModalContext.Provider value={api}>
            {props.children}
            <Show when={current()}>
                {(req) => (
                    <div
                        class="tab-modal-overlay"
                        role="presentation"
                        // eslint-disable-next-line jsx-a11y/no-autofocus
                        tabIndex={-1}
                        onClick={handleBackdropClick}
                        onKeyDown={handleOverlayKeyDown}
                    >
                        <div class="tab-modal-backdrop" />
                        <div
                            class="tab-modal-panel"
                            role="dialog"
                            aria-modal="true"
                            onClick={(e) => e.stopPropagation()}
                        >
                            {renderRequest(req(), api)}
                        </div>
                    </div>
                )}
            </Show>
        </TabModalContext.Provider>
    );
};

// ── Render dispatch ──────────────────────────────────────────────────────────
//
// One branch per request `kind`. The panel's `onSubmit` re-throws on
// failure so the panel can surface the error in its own UI; on success
// the layer closes itself.

function renderRequest(req: TabModalRequest, api: TabModalApi): JSX.Element {
    switch (req.kind) {
        case "launch-agent":
            return (
                <AgentLaunchModalPanel
                    agent={req.agent}
                    onCancel={api.close}
                    onSubmit={async (overrides) => {
                        await req.onSubmit(overrides);
                        api.close();
                    }}
                />
            );
    }
}
