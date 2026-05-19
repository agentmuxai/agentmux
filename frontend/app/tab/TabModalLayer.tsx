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

import { createEffect, createMemo, createSignal, onCleanup, onMount, Show, type Accessor, type Component, type JSX } from "solid-js";

import { usePaneOverlay } from "@/app/platform/pane-overlay";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { AgentLaunchModalPanel } from "@/app/view/agent/components/AgentLaunchModal";
import { AgentInstallModalPanel } from "@/app/view/agent/components/AgentInstallModal";
import { AgentPrereqModalPanel } from "@/app/view/agent/components/AgentPrereqModal";
import { AgentNewIdentityModalPanel } from "@/app/view/agent/components/AgentNewIdentityModal";
import "@/app/view/agent/components/AgentPrereqModal.scss";
import "@/app/view/agent/components/AgentNewBundleModal.scss";

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
//
// ResizeObserver supplement: inactive tabs go display:none on their
// container, which makes the overlay's bounding rect collapse to zero.
// usePaneOverlay only refreshes on window.resize, so we observe the
// element for size changes and dispatch a synthetic resize to flush the
// now-zero rect to the backend, clearing the stale clip.
const PaneOverlayClip: Component<{ getEl: Accessor<HTMLElement | null | undefined> }> = (p) => {
    usePaneOverlay(p.getEl);
    onMount(() => {
        const el = p.getEl();
        if (!el) return;
        const ro = new ResizeObserver(() => window.dispatchEvent(new Event("resize")));
        ro.observe(el);
        onCleanup(() => ro.disconnect());
    });
    return null;
};

export const TabModalLayer: Component<TabModalLayerProps> = (props) => {
    const [current, setCurrent] = createSignal<TabModalRequest | null>(null);
    const [submitting, setSubmitting] = createSignal(false);
    // Paint gate — see SPEC_MODAL_PAINT_GATE_2026_05_18.md.
    //
    // Outer gate (`ready`): arms ONLY on a null→non-null transition,
    // i.e. cold open of a modal session. `tabModal.replace()` keeps
    // the backdrop + outer panel mounted, so we must NOT toggle ready
    // back to false on replace — that would briefly hide the
    // persistent shell mid-crossfade (reagent P1 + codex P2 on PR
    // #900). Stays true through replaces; resets only on close.
    //
    // Inner gate (`contentReady`): re-arms on every request identity
    // change so each replace swaps content through a hidden frame
    // before the crossfade keyframe runs.
    //
    // Both gates schedule rAF×2 (waits for one full paint cycle to
    // commit) with a 200ms failsafe in case the renderer is suspended
    // (background tab). Both rAF handles AND the failsafe are
    // cancelled in onCleanup so stale callbacks from a prior arm
    // don't flip the new arm early (reagent P2 on #900).
    const [ready, setReady] = createSignal(false);
    const [contentReady, setContentReady] = createSignal(false);

    const armGate = (setGate: (v: boolean) => void): (() => void) => {
        setGate(false);
        let innerRaf: number | null = null;
        const failsafe = setTimeout(() => setGate(true), 200);
        const outerRaf = requestAnimationFrame(() => {
            innerRaf = requestAnimationFrame(() => {
                clearTimeout(failsafe);
                setGate(true);
            });
        });
        return () => {
            clearTimeout(failsafe);
            cancelAnimationFrame(outerRaf);
            if (innerRaf != null) cancelAnimationFrame(innerRaf);
        };
    };

    // Outer gate: arm only on cold open (null→non-null). On replace()
    // (non-null → non-null) the gate stays as-is so a mid-crossfade
    // doesn't briefly hide the persistent shell.
    //
    // Edge case (codex P2 / reagent P2 on PR #900): if replace() fires
    // BEFORE the cold-open rAF×2 / 200ms failsafe has flipped ready to
    // true, Solid runs this effect's previous onCleanup — which cancels
    // both rAFs and the failsafe — then re-runs the body. If we
    // unconditionally bail on `prevWasOpen`, no callback survives to
    // flip ready, and the overlay stays opacity:0 forever. Fix: only
    // bail when the gate has actually fired. If the gate is still
    // in-flight at the moment of replace(), re-arm it.
    let prevWasOpen = false;
    createEffect(() => {
        const isOpen = current() != null;
        if (!isOpen) { prevWasOpen = false; setReady(false); return; }
        if (prevWasOpen && ready()) return;   // gate already fired — leave it alone
        prevWasOpen = true;
        const cleanup = armGate(setReady);
        onCleanup(cleanup);
    });

    // Inner gate: re-arm on every request identity change.
    createEffect(() => {
        if (current() == null) { setContentReady(false); return; }
        const cleanup = armGate(setContentReady);
        onCleanup(cleanup);
    });

    // Guard close: ESC and backdrop click are blocked while a submit RPC is
    // in-flight so the user can't lose error feedback or trigger a duplicate launch.
    const safeClose = () => {
        if (!submitting()) setCurrent(null);
    };

    const api: TabModalApi = {
        open: (req) => { setSubmitting(false); setCurrent(req); },
        replace: (next) => {
            // Identical to `open` at the signal level — the visual
            // difference (cold open plays the backdrop fade-in + panel
            // pop-in; warm replace fires only the content keyframe) is
            // emergent from <Show>'s mount state. Reagent caught this:
            // splitting the assignment into two branches was dead code.
            // See SPEC_MODAL_TRANSITIONS_2026_05_18.md §3.3.
            setSubmitting(false);
            setCurrent(next);
        },
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
                {(reqAcc) => {
                    // Backdrop + outer panel persist for the lifetime of
                    // a "modal session" (one or more chained requests via
                    // `replace`). The inner keyed <Show> remounts the
                    // content subtree on each request swap, triggering
                    // the content-fade animation while the shell stays
                    // put — no backdrop flicker, no entrance-pop replay.
                    let overlayRef: HTMLDivElement | undefined;
                    const meta = createMemo(() => renderRequest(reqAcc(), api, setSubmitting));
                    return (
                        <div
                            class="tab-modal-overlay"
                            data-ready={ready() ? "" : undefined}
                            data-content-ready={contentReady() ? "" : undefined}
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
                                aria-label={meta().label}
                                onClick={(e) => e.stopPropagation()}
                            >
                                <Show keyed when={reqAcc()}>
                                    {(_req) => (
                                        <div class="tab-modal-content">
                                            {meta().panel}
                                        </div>
                                    )}
                                </Show>
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
): { label: string; panel: JSX.Element } {
    switch (req.kind) {
        case "launch-agent":
            return {
                label: `Launch ${req.agent.name}`,
                panel: (
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
                        preselectedIdentityId={req.preselectedIdentityId}
                        preselectedMemoryId={req.preselectedMemoryId}
                        onRequestNewIdentity={req.onRequestNewIdentity}
                        onRequestNewMemory={req.onRequestNewMemory}
                    />
                ),
            };
        case "new-identity":
            return {
                label: "New Identity",
                panel: (
                    <AgentNewIdentityModalPanel
                        initialName={req.initialName}
                        // The layer owns the RPC + chaining so its
                        // `submitting()` flag (which gates safeClose)
                        // tracks the in-flight call. Mirrors the
                        // launch-agent dispatch above — reagent P1 on
                        // PR #911.
                        onSubmit={async ({ name, description }) => {
                            setSubmitting(true);
                            try {
                                const id = crypto.randomUUID();
                                const now = Date.now();
                                const bundle = await RpcApi.UpsertIdentityBundleCommand(
                                    TabRpcClient,
                                    {
                                        id,
                                        name,
                                        description,
                                        is_blank: false,
                                        created_at: now,
                                        updated_at: now,
                                    },
                                );
                                setSubmitting(false);
                                // Caller's onCreated does tabModal.replace
                                // back to Launch with the new id
                                // preselected — that's what unmounts this
                                // panel. We don't `api.close()` here.
                                req.onCreated(bundle.id, bundle.name);
                            } catch (e) {
                                setSubmitting(false);
                                throw e;
                            }
                        }}
                        // Caller's onCancel does tabModal.replace back to
                        // Launch with the prior selection intact. Running
                        // api.close() afterward would nullify that replace
                        // (both run synchronously, last write wins) and
                        // exit the launch flow — reagent P1 on PR #910.
                        onCancel={req.onCancel}
                    />
                ),
            };
        case "agent-prereqs":
            return {
                label: `Install required tools for ${req.agent.name}`,
                panel: (
                    <AgentPrereqModalPanel
                        agent={req.agent}
                        missing={req.missing}
                        onRefresh={() => req.onRefresh()}
                        onProceed={() => req.onProceed()}
                        onCancel={() => {
                            req.onCancel();
                            api.close();
                        }}
                    />
                ),
            };
        case "install-agent":
            return {
                label: `Install ${req.agent.name}`,
                panel: (
                    <AgentInstallModalPanel
                        agent={req.agent}
                        onCancel={api.close}
                        onInstalled={(continueToLaunch: boolean) => {
                            // Hand off to the picker — it owns whether
                            // to call `tabModal.replace(launchReq)`
                            // (continueToLaunch=true) or `tabModal.close()`
                            // (continueToLaunch=false). Don't tear down
                            // the shell here — that would break the
                            // install→launch crossfade for the chain
                            // path. SPEC_MODAL_TRANSITIONS_2026_05_18.md.
                            req.onInstalled(continueToLaunch);
                        }}
                    />
                ),
            };
    }
}
