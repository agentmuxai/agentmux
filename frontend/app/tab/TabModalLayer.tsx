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

import { createSignal, Show, type Accessor, type Component, type JSX } from "solid-js";

import { usePaneOverlay } from "@/app/platform/pane-overlay";
import { AgentLaunchModalPanel } from "@/app/view/agent/components/AgentLaunchModal";

import { TabModalContext, type TabModalApi, type TabModalRequest } from "./tab-modal";
import "./tab-modal.scss";

interface TabModalLayerProps {
    children: JSX.Element;
}

// Registers the overlay element with the backend pane-clip system so
// native browser-pane HWNDs cut a transparent hole matching the overlay
// rect. Without this, hardware-windowed panes composite above HTML
// regardless of CSS z-index and render on top of the modal.
// Mirrors the ModalPaneOverlayClip pattern in modal-v2.tsx.
const PaneOverlayClip: Component<{ getEl: Accessor<HTMLElement | null | undefined> }> = (p) => {
    usePaneOverlay(p.getEl);
    return null;
};

export const TabModalLayer: Component<TabModalLayerProps> = (props) => {
    const [current, setCurrent] = createSignal<TabModalRequest | null>(null);
    const [submitting, setSubmitting] = createSignal(false);

    // Guard close: ESC and backdrop click are blocked while a submit RPC is
    // in-flight so the user can't lose error feedback or trigger a duplicate launch.
    const safeClose = () => {
        if (!submitting()) setCurrent(null);
    };

    const api: TabModalApi = {
        open: (req) => { setSubmitting(false); setCurrent(req); },
        close: safeClose,
        current,
    };

    const handleOverlayKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Escape") {
            e.stopPropagation();
            safeClose();
        }
    };

    // `display:contents` is layout-transparent — TileLayout still sees
    // TabContent's flex-row container as its parent. `inert` makes the
    // subtree non-interactive when a modal is open, trapping focus inside
    // the overlay panel without the layout disruption a normal div would cause.
    return (
        <TabModalContext.Provider value={api}>
            <div style="display:contents" inert={current() != null || undefined}>
                {props.children}
            </div>
            <Show when={current()}>
                {(req) => {
                    let overlayRef: HTMLDivElement | undefined;
                    return (
                        <div
                            class="tab-modal-overlay"
                            ref={(el) => { overlayRef = el; }}
                            role="presentation"
                            tabIndex={-1}
                            onKeyDown={handleOverlayKeyDown}
                        >
                            <PaneOverlayClip getEl={() => overlayRef} />
                            {/* Click on backdrop (not panel) closes. Handler is on the
                                backdrop element itself — the backdrop covers the full
                                overlay area, so e.target === e.currentTarget on the
                                overlay would never fire. */}
                            <div class="tab-modal-backdrop" onClick={safeClose} />
                            <div
                                class="tab-modal-panel"
                                role="dialog"
                                aria-modal="true"
                                onClick={(e) => e.stopPropagation()}
                            >
                                {renderRequest(req(), api, setSubmitting)}
                            </div>
                        </div>
                    );
                }}
            </Show>
        </TabModalContext.Provider>
    );
};

// ── Render dispatch ──────────────────────────────────────────────────────────

function renderRequest(
    req: TabModalRequest,
    api: TabModalApi,
    setSubmitting: (v: boolean) => void,
): JSX.Element {
    switch (req.kind) {
        case "launch-agent":
            return (
                <AgentLaunchModalPanel
                    agent={req.agent}
                    onCancel={api.close}
                    onSubmit={async (overrides) => {
                        setSubmitting(true);
                        try {
                            await req.onSubmit(overrides);
                            setSubmitting(false);
                            api.close();
                        } catch (e) {
                            setSubmitting(false);
                            throw e;
                        }
                    }}
                />
            );
    }
}
