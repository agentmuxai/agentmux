// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createSignal, onCleanup, Show, type JSX } from "solid-js";
import { invokeCommand } from "@/app/platform/ipc";
import { ModalLayer } from "@/element/ModalLayer";
import { useModalLayer } from "@/element/modal-layer";
import { BrainSpinner } from "@/app/element/BrainSpinner";
import { atoms } from "@/store/global";
import type { BrowserViewModel } from "./browser-model";
import { usePaneRectSync } from "./use-pane-rect-sync";
import { useFreezeFrame } from "./use-freeze-frame";
import { useBrowserAuth } from "./use-browser-auth";
import { BrowserNavBar } from "./browser-nav-bar";
import "./browser-view.scss";

// Matches BrainSpinner.scss's `.is-fading` transition duration — the DOM
// node stays mounted this long after loadingAtom() flips false so the
// opacity fade actually plays before unmount removes it.
const LOADING_SPINNER_FADE_MS = 200;

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
export function BrowserViewComponent(props: ViewComponentProps<BrowserViewModel>): JSX.Element {
    return (
        <ModalLayer scope="pane">
            <BrowserViewInner model={props.model} />
        </ModalLayer>
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
    // 200 Hz focus bounce — ipc.rs:424-428 documents this).
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
                        invokeCommand("browser_pane_focus", { block_id: model.blockId }).catch(() => {});
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
            </div>
        </div>
    );
}
