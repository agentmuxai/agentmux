// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createSignal, For, onCleanup, Show, type JSX } from "solid-js";
import { isAnyUnderway } from "@/app/drag/drag-session";
import { hostHas } from "@/app/host/host-caps";
import { getApi } from "@/app/store/app-api";
import { usePaneOverlay } from "@/app/platform/pane-overlay";
import { ModalLayer } from "@/element/ModalLayer";
import { useModalLayer } from "@/element/modal-layer";
import { BrainSpinner } from "@/app/element/BrainSpinner";
import { atoms } from "@/store/global";
import type { BrowserViewModel } from "./browser-model";
import { usePaneRectSync } from "./use-pane-rect-sync";
import { useDragSnapshot, type DragSnapshot } from "./use-drag-snapshot";
import { useFreezeFrame } from "./use-freeze-frame";
import { useBrowserAuth } from "./use-browser-auth";
import { Button } from "@/app/element/ui";
import { BrowserNavBar } from "./browser-nav-bar";
import "./browser-view.scss";

// Matches BrainSpinner.scss's `.is-fading` transition duration — the DOM
// node stays mounted this long after loadingAtom() flips false so the
// opacity fade actually plays before unmount removes it.
const LOADING_SPINNER_FADE_MS = 200;

// How long the drag catcher waits for the page snapshot before opening the
// hole anyway: a drop that works beats a pretty one. Captures measured
// 50-90ms on Windows, and are usually prewarmed before the drag starts
// (use-drag-snapshot.ts).
const DRAG_SNAPSHOT_CAP_MS = 500;

// The in-app (pragmatic) drags: a whole pane, a Window Tab, a Pane Tab, an
// Editor or Media document tab. The page is a native window drawn above the
// DOM and can't see them, so the catcher below is shown from each one's start
// until its source releases it.
const ELEMENT_DRAG_KINDS = ["tile", "window-tab", "pane-tab", "doc-tab"] as const;

/**
 * Covers the page for the length of an in-app drag (a pane, a Pane Tab, a
 * Window Tab) and opens a hole through the native page. Without it the OS
 * handed the drag to the page, which refused it: a circle-slash cursor, and
 * dropping a tab onto a browser pane did nothing because the pane's own drop
 * target never saw the drag.
 *
 * The hole alone showed the bare placeholder (the page turned grey mid-drag),
 * so the catcher first shows a snapshot of the page and only opens the hole
 * once that has painted, or after DRAG_SNAPSHOT_CAP_MS. The snapshot is
 * static; a drag is short.
 */
function BrowserDragCatcher(props: { takeSnapshot: () => Promise<DragSnapshot | null> }): JSX.Element {
    const [snapshot, setSnapshot] = createSignal<DragSnapshot | null>(null);
    const [holeOpen, setHoleOpen] = createSignal(false);
    const cap = setTimeout(() => setHoleOpen(true), DRAG_SNAPSHOT_CAP_MS);
    let disposed = false;
    onCleanup(() => {
        disposed = true;
        clearTimeout(cap);
    });
    void props.takeSnapshot().then((s) => {
        if (disposed) return;
        if (s) setSnapshot(s);
        else setHoleOpen(true);
    });
    return (
        <div class="browser-drag-catcher">
            <Show when={snapshot()}>
                <img
                    class="browser-freeze-snapshot"
                    alt=""
                    src={snapshot()!.src}
                    style={snapshot()!.style}
                    // One frame after load, so the image is on screen before
                    // the page is cut away from above it.
                    onLoad={() => requestAnimationFrame(() => setHoleOpen(true))}
                />
            </Show>
            <Show when={holeOpen()}>
                <BrowserDragHole />
            </Show>
        </div>
    );
}

/** The hole itself: registered through `usePaneOverlay` rather than
 *  `data-pane-overlay` so it also works off Windows. */
function BrowserDragHole(): JSX.Element {
    let el: HTMLDivElement | undefined;
    usePaneOverlay(() => el);
    return <div class="browser-drag-hole" ref={el} />;
}

/**
 * Pane-scope modal host. Wraps the browser-pane content in a
 * `<ModalLayer scope="pane">` so any `useModalLayer()` call inside
 * resolves to THIS layer rather than the outer tab-scope one
 * (from `tabcontent.tsx`). The HTTP Basic / Digest auth modal that
 * fires on a 401-protected URL then locks only this pane —
 * everything else in the tab (sibling panes, tab bar, title bar)
 * stays interactive.
 *
 * Split into a thin outer + inner so the inner's `useModalLayer()`
 * call (line below) resolves against the wrapper's context, not the
 * caller's. Solid's hook resolves up the JSX tree at execution time,
 * so the consumer must be a CHILD of the provider — putting the
 * `useModalLayer()` call in the same function body as the
 * `<ModalLayer>` JSX would have it resolve to the outer (tab) layer
 * instead.
 * SPEC_LAUNCH_MODAL_PANE_SCOPE_2026_05_25.md §5 (browser-auth follow-up).
 */
export function BrowserViewComponent(props: { model: BrowserViewModel }): JSX.Element {
    // A browser pane is a native browser drawn by the host. A host without
    // them (HostCaps.nativeBrowserPane) gets a notice, never a pane that
    // issues commands nothing will answer.
    return (
        <Show when={hostHas("nativeBrowserPane")} fallback={<BrowserPaneUnavailable />}>
            <ModalLayer scope="pane">
                <BrowserViewInner model={props.model} />
            </ModalLayer>
        </Show>
    );
}

function BrowserPaneUnavailable(): JSX.Element {
    return (
        <div class="browser-view browser-pane-unavailable">
            <p>Browser panes need the AgentMux desktop app.</p>
        </div>
    );
}

function BrowserViewInner(props: { model: BrowserViewModel }): JSX.Element {
    const model = props.model;
    const modalLayer = useModalLayer();
    const _diagTag = `[browser-pane:diag][${model.blockId.slice(0, 7)}]`;
    const diag = (msg: string): void => { console.log(`${_diagTag} ${msg}`); };
    // Captured once at mount — same window_label that createPane uses for
    // browser_pane_create. Required so main_window_focus reclaims OS focus
    // to the WINDOW that sent the IPC (otherwise the host's handler
    // defaults to "main" and misroutes in multi-window setups, creating a
    // 200 Hz focus bounce — the `main_window_focus` arm in ipc.rs documents
    // the misrouting).
    const windowLabel =
        new URLSearchParams(window.location.search).get("windowLabel") ?? "main";
    diag(`view-mount window_label=${windowLabel} initial-urlAtom=${JSON.stringify(model.urlAtom())}`);

    let placeholderRef: HTMLDivElement | undefined;

    // Construction ORDER matters: freeze-frame reads paneRect()/paneCreated()
    // owned by the rect-sync hook, so rect-sync must be built first.
    const rectSync = usePaneRectSync({
        model,
        placeholderRef: () => placeholderRef,
        windowLabel,
        diag,
    });
    const freeze = useFreezeFrame({
        model,
        placeholderRef: () => placeholderRef,
        paneRect: rectSync.paneRect,
        paneCreated: rectSync.paneCreated,
        diag,
    });
    const dragSnapshot = useDragSnapshot({
        model,
        placeholderRef: () => placeholderRef,
        paneRect: rectSync.paneRect,
        paneCreated: rectSync.paneCreated,
        diag,
    });
    useBrowserAuth({ model, modalLayer, diag });

    // Loading-brain overlay (SPEC_BROWSER_PANE_LOADING_BRAIN_INDICATOR_2026_07_11.md
    // §4.3). `model.loadingAtom()` is the source of truth; these two signals
    // exist only to hold the BrainSpinner mounted for the CSS fade-out
    // duration after loading finishes — BrainSpinner's own contract is "the
    // caller owns unmounting it after the transition ends" (see its doc
    // comment), so a plain `<Show when={model.loadingAtom()}>` would yank it
    // out instantly with no fade.
    const [spinnerMounted, setSpinnerMounted] = createSignal(false);
    const [spinnerFading, setSpinnerFading] = createSignal(false);
    let spinnerFadeTimeout: ReturnType<typeof setTimeout> | null = null;
    createEffect(() => {
        if (model.loadingAtom()) {
            if (spinnerFadeTimeout) {
                clearTimeout(spinnerFadeTimeout);
                spinnerFadeTimeout = null;
            }
            setSpinnerFading(false);
            setSpinnerMounted(true);
            return;
        }
        if (!spinnerMounted()) return;
        // prefersReducedMotion: BrainSpinner shows/hides instantly (no CSS
        // transition) in that mode, so holding the node mounted for the
        // normal fade duration would just be a pointless delay — unmount now.
        if (atoms.prefersReducedMotionAtom()) {
            setSpinnerMounted(false);
            return;
        }
        setSpinnerFading(true);
        spinnerFadeTimeout = setTimeout(() => {
            spinnerFadeTimeout = null;
            setSpinnerFading(false);
            setSpinnerMounted(false);
        }, LOADING_SPINNER_FADE_MS);
    });
    onCleanup(() => {
        if (spinnerFadeTimeout) clearTimeout(spinnerFadeTimeout);
    });

    // Layer 2, SPEC_BROWSER_PANE_LOADING_INDICATOR_FLICKER_2026_08_17.md:
    // once the pane has painted real content at least once, a later loading
    // flip (reload, back/forward, a redirect chain — layer 1 deliberately
    // doesn't suppress those, they're real navigations) no longer needs to
    // cover a blank gap. Hiding the whole native pane HWND for it would just
    // flash the already-visible page away and back for no reason — that's
    // the reported bug. Tracks a real true→false `loadingAtom()` transition
    // (not the initial read, which is `false` only before the constructor's
    // `Navigate` dispatch takes effect); never resets within this view's
    // lifetime — a fresh pane construction gets its own fresh signal.
    const [hasPaintedOnce, setHasPaintedOnce] = createSignal(false);
    let wasLoading = false;
    createEffect(() => {
        const loading = model.loadingAtom();
        if (wasLoading && !loading) setHasPaintedOnce(true);
        wasLoading = loading;
    });

    return (
        <div class="browser-view">
            <BrowserNavBar
                model={model}
                windowLabel={windowLabel}
                diag={diag}
                paneCreated={rectSync.paneCreated}
                createPane={rectSync.createPane}
            />

            <Show when={model.attentionAtom()}>
                {(a) => (
                    <div class="browser-attention" role="alertdialog" aria-live="assertive">
                        <div class="browser-attention-head">
                            <i class="fa-solid fa-hand" aria-hidden="true" />
                            <Show
                                when={a().kind === "navigation"}
                                fallback={
                                    <span>
                                        <b>{a().agent || "An agent"}</b>{" "}
                                        {a().kind === "handoff" ? "needs you: " : "wants to: "}
                                        {a().kind === "handoff" ? a().reason : a().what}
                                    </span>
                                }
                            >
                                <span>
                                    This pane is limited to the sites it was opened for. The page wants to{" "}
                                    {a().popup ? "open a window at " : "go to "}
                                    <b>{a().origin}</b>.
                                </span>
                            </Show>
                        </div>
                        <Show when={a().kind === "navigation" && a().url}>
                            <div class="browser-attention-target" title={a().url}>
                                {a().url}
                            </div>
                        </Show>
                        <Show when={a().kind === "navigation"}>
                            <div class="browser-attention-target">
                                {a().popup
                                    ? "Allow adds this site to the pane's list; then click again to open the window."
                                    : "Allow adds this site to the pane's list and goes there. A form's data isn't sent again."}
                            </div>
                        </Show>
                        <Show when={a().window}>
                            <div class="browser-attention-target">In its popup window: {a().window}</div>
                        </Show>
                        <Show when={a().kind === "approval" && (a().fields?.length ?? 0) > 0}>
                            <table class="browser-attention-fields">
                                <tbody>
                                    <For each={a().fields}>
                                        {(f) => (
                                            <tr>
                                                <td>{f[0]}</td>
                                                <td>{f[1]}</td>
                                            </tr>
                                        )}
                                    </For>
                                </tbody>
                            </table>
                        </Show>
                        <Show when={a().kind === "approval" && a().action}>
                            <div class="browser-attention-target">Sends to {a().action}</div>
                        </Show>
                        <div class="browser-attention-actions">
                            <Button
                                tone="accent"
                                density="compact"
                                onClick={() => model.resolveAttention(a().kind === "handoff" ? "done" : "approve").catch(() => {})}
                            >
                                {a().kind === "handoff" ? "Done" : a().kind === "navigation" ? "Allow" : "Approve"}
                            </Button>
                            <Button density="compact" onClick={() => model.resolveAttention("cancel").catch(() => {})}>
                                {a().kind === "navigation" ? "Block" : "Cancel"}
                            </Button>
                        </div>
                    </div>
                )}
            </Show>
            <Show when={model.popupWindowsAtom().length > 0}>
                <div class="browser-popup-windows" role="list" aria-label="Popup windows">
                    <For each={model.popupWindowsAtom()}>
                        {(w) => (
                            <div class="browser-popup-window" role="listitem">
                                <i class="fa-solid fa-window-restore" aria-hidden="true" />
                                <span class="browser-popup-window-url" title={w.url}>
                                    Popup window: {w.url}
                                </span>
                                <Button
                                    density="compact"
                                    title="Bring this popup window to the front"
                                    onClick={() => model.showPopup(w.id).catch(() => {})}
                                >
                                    Show
                                </Button>
                                <Button
                                    density="compact"
                                    title="Close this popup window"
                                    onClick={() => model.closePopup(w.id).catch(() => {})}
                                >
                                    Close
                                </Button>
                            </div>
                        )}
                    </For>
                </div>
            </Show>
            <Show when={model.popupFromAtom()}>
                {(from) => (
                    <div class="browser-popup-from" role="note">
                        <i class="fa-solid fa-window-restore" aria-hidden="true" />
                        <span>
                            Popup from <b>{from()}</b>
                        </span>
                    </div>
                )}
            </Show>
            <Show when={model.driverAgentAtom()}>
                {(agent) => (
                    <div class="browser-driven-by" role="status">
                        <i class="fa-solid fa-robot" aria-hidden="true" />
                        <span>
                            Driven by <b>{agent()}</b>
                        </span>
                        <Show when={model.allowedOriginsAtom().length > 0}>
                            <span
                                class="browser-limited-to"
                                title={`Limited to: ${model.allowedOriginsAtom().join(", ")}`}
                            >
                                Limited to {model.allowedOriginsAtom().length}{" "}
                                {model.allowedOriginsAtom().length === 1 ? "site" : "sites"}
                            </span>
                        </Show>
                        <Button
                            density="compact"
                            class="browser-take-over"
                            title="End the agent's control of this pane; its next action here will fail."
                            onClick={() => model.takeOver().catch(() => {})}
                        >
                            Take over
                        </Button>
                    </div>
                )}
            </Show>
            <Show when={model.errorAtom()}>
                <div class="browser-error">{model.errorAtom()}</div>
            </Show>

            <div
                class="browser-placeholder"
                ref={placeholderRef}
                onMouseDown={() => {
                    // User clicked into the pane — explicitly hand Windows-level
                    // keyboard focus to the pane HWND so subsequent keystrokes
                    // and mouse-wheel events route there.
                    //
                    // We used to grab focus on onMouseEnter (hover), but that
                    // created a loop: clicking the address bar released focus,
                    // then the cursor drifting back over the pane (inevitable —
                    // the address bar is right above it) re-grabbed focus
                    // before the user could type. Hover-focus is nicer for
                    // scroll-without-click, but it breaks keyboard routing so
                    // aggressively that the trade-off doesn't pay. Explicit
                    // click is the clear user intent.
                    if (rectSync.paneCreated() && !model.closed) {
                        getApi().browserPanes.focus(model.blockId).catch(() => {});
                    }
                }}
            >
                <Show when={!model.urlAtom() && !rectSync.paneCreated()}>
                    <div class="browser-empty">
                        <div class="browser-empty-icon">{"🌐"}</div>
                        <div class="browser-empty-text">Enter a URL above to browse</div>
                    </div>
                </Show>
                <Show when={freeze.freezeSnapshot()}>
                    <img
                        class="browser-freeze-snapshot"
                        alt=""
                        src={freeze.freezeSnapshot()!}
                        style={freeze.freezeStyle()}
                    />
                </Show>

                {/* `data-pane-overlay`: pane-overlay-auto.ts auto-discovers this
                    element and punches a matching hole through the native
                    browser-pane HWND so it's visible above it (the HWND paints
                    above DOM regardless of CSS z-index — the "airspace problem",
                    SPEC_PANE_OVERLAY_AUTO_CLIP_2026_05_11.md). No manual overlay
                    registration needed — same mechanism modals/menus/tooltips
                    already use.

                    The fade-out opacity is applied to THIS wrapper (via
                    is-fading below), not just to BrainSpinner's own internal
                    fade — pane-overlay-auto.ts's isOverlayElementVisible()
                    reads computed opacity on the tagged data-pane-overlay
                    element itself. Fading only BrainSpinner's inner div would
                    make it look faded while this outer element stayed at
                    opacity:1, so the clip hole wouldn't lift until unmount —
                    losing the "fade and un-punch together" behavior this
                    design relies on. BrainSpinner's own `fading` prop is
                    unnecessary here since opacity on this wrapper already
                    fades everything nested inside it.

                    Layer 2 (SPEC_BROWSER_PANE_LOADING_INDICATOR_FLICKER_2026_08_17.md)
                    branches this on `hasPaintedOnce()`: full-pane coverage
                    only for the FIRST load, when there's genuinely nothing
                    behind it yet to hide. Once the pane has painted once, a
                    later loading flip gets a small corner badge instead —
                    its `data-pane-overlay` rect is tiny, so even a flip
                    layer 1 doesn't catch can only punch a small hole, never
                    hide the whole visible page. */}
                <Show when={spinnerMounted()}>
                    <Show
                        when={!hasPaintedOnce()}
                        fallback={
                            <div
                                class="browser-loading-badge"
                                classList={{ "is-fading": spinnerFading() }}
                                data-pane-overlay
                            >
                                <BrainSpinner class="browser-loading-badge-spinner" />
                            </div>
                        }
                    >
                        <div class="browser-loading-overlay" classList={{ "is-fading": spinnerFading() }} data-pane-overlay>
                            <BrainSpinner />
                        </div>
                    </Show>
                </Show>
                <Show when={isAnyUnderway(ELEMENT_DRAG_KINDS)}>
                    <BrowserDragCatcher takeSnapshot={dragSnapshot.take} />
                </Show>
            </div>
        </div>
    );
}
