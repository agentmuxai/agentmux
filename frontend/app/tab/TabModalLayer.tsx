// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * TabModalLayer — per-tab modal host.
 *
 * Wraps a tab's tile layout, provides a context for opening tab-scoped
 * modals, and renders the overlay + panel inline (no Portal). Because
 * it lives inside `<TabContent>`, switching tabs hides the modal via
 * the existing `display:none` on inactive tab content. The top tab bar
 * is outside this layer and stays interactive.
 *
 * Rationale:
 *   - Form input lag in the original launch modal came largely from a
 *     Portal-mounted full-window backdrop blur and from the modal
 *     sharing a reactive scope with the agent picker. Both are gone:
 *     the backdrop is scoped to the tab area, and the modal renders in
 *     a sibling slot of the tile layout, isolated from the picker tree.
 *   - The user's mental model is "the modal is bound to the tab I
 *     opened it from"; rendering inside `<TabContent>` matches that.
 *   - The legacy global `Modal` (modal-v2) stays for window-level
 *     dialogs (command palette, about, backend prompts). This layer is
 *     additive, not a replacement.
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
    // click inside the form doesn't reach the backdrop. ESC closes via
    // the layer's keyDown handler — caught here so unrelated ESC users
    // in the tile layout aren't pre-empted by a global listener.
    const handleBackdropClick = (e: MouseEvent) => {
        if (e.target === e.currentTarget) {
            api.close();
        }
    };

    const handleKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Escape" && current() != null) {
            e.stopPropagation();
            api.close();
        }
    };

    return (
        <TabModalContext.Provider value={api}>
            <div
                class="tab-modal-layer"
                data-modal-open={current() != null ? "true" : undefined}
                onKeyDown={handleKeyDown}
            >
                <div
                    class="tab-modal-content"
                    inert={current() != null || undefined}
                    aria-hidden={current() != null ? "true" : undefined}
                >
                    {props.children}
                </div>
                <Show when={current()}>
                    {(req) => (
                        <div class="tab-modal-overlay" role="presentation" onClick={handleBackdropClick}>
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
            </div>
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
