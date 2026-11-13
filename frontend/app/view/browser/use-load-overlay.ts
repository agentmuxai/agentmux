// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createSignal, on, onCleanup, type Accessor } from "solid-js";
import { scaledMs, shouldReduceMotion } from "@/app/motion/motion";
import { DURATION_BASE_MS } from "@/app/motion/tokens";
import type { BrowserViewModel } from "./browser-model";
import { wakeEndsNow } from "./hibernation";
import type { PaneSleep } from "./use-pane-sleep";

// The overlay's `.is-fading` transition (browser-view.scss) is `--duration-base`
// scaled by the Animations setting. The DOM node stays mounted this long after
// loadingAtom() flips false, so the fade plays before unmount removes it.
const loadingSpinnerFadeMs = () => scaledMs(DURATION_BASE_MS);

/**
 * Loading-brain overlay (SPEC_BROWSER_PANE_LOADING_BRAIN_INDICATOR_2026_07_11.md
 * §4.3). `model.loadingAtom()` is the source of truth; the two signals
 * exist only to hold the BrainSpinner mounted for the CSS fade-out
 * duration after loading finishes — BrainSpinner's own contract is "the
 * caller owns unmounting it after the transition ends" (see its doc
 * comment), so a plain `<Show when={model.loadingAtom()}>` would yank it
 * out instantly with no fade.
 */
function useSpinnerFade(model: BrowserViewModel) {
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
        // When the motion policy reduces `transition`, BrainSpinner
        // shows/hides instantly (no CSS transition), so holding the node
        // mounted for the normal fade duration would just be a pointless
        // delay — unmount now.
        if (shouldReduceMotion("transition")) {
            setSpinnerMounted(false);
            return;
        }
        setSpinnerFading(true);
        spinnerFadeTimeout = setTimeout(() => {
            spinnerFadeTimeout = null;
            setSpinnerFading(false);
            setSpinnerMounted(false);
        }, loadingSpinnerFadeMs());
    });
    onCleanup(() => {
        if (spinnerFadeTimeout) clearTimeout(spinnerFadeTimeout);
    });
    return { spinnerMounted, spinnerFading };
}

/**
 * Layer 2, SPEC_BROWSER_PANE_LOADING_INDICATOR_FLICKER_2026_08_17.md:
 * once the pane has painted real content at least once, a later loading
 * flip (reload, back/forward, a redirect chain — layer 1 deliberately
 * doesn't suppress those, they're real navigations) no longer needs to
 * cover a blank gap. Hiding the whole native pane HWND for it would just
 * flash the already-visible page away and back for no reason — that's
 * the reported bug. Tracks a real true→false `loadingAtom()` transition
 * (not the initial read, which is `false` only before the constructor's
 * `Navigate` dispatch takes effect); never resets within this view's
 * lifetime — a fresh pane construction gets its own fresh signal.
 */
function useHasPaintedOnce(model: BrowserViewModel): Accessor<boolean> {
    const [hasPaintedOnce, setHasPaintedOnce] = createSignal(false);
    let wasLoading = false;
    createEffect(() => {
        const loading = model.loadingAtom();
        if (wasLoading && !loading) setHasPaintedOnce(true);
        wasLoading = loading;
    });
    return hasPaintedOnce;
}

export function useLoadingOverlay(model: BrowserViewModel) {
    const fade = useSpinnerFade(model);
    return { ...fade, hasPaintedOnce: useHasPaintedOnce(model) };
}

export type LoadingOverlay = ReturnType<typeof useLoadingOverlay>;

/**
 * How long a load takes from the page's creation (or the wake's start),
 * for the card's next estimate; a wake ends when it has loaded.
 */
function useLoadClock(p: { model: BrowserViewModel; sl: PaneSleep; paneCreated: Accessor<boolean>; diag: (msg: string) => void }) {
    const { model, sl, paneCreated, diag } = p;
    let loadClockFrom = 0;
    let sawLoading = false;
    createEffect(
        on(paneCreated, (created) => {
            if (!created) {
                loadClockFrom = 0;
                return;
            }
            loadClockFrom = sl.sleeping() && sl.wakeStartedAt() > 0 ? sl.wakeStartedAt() : performance.now();
            sawLoading = model.loadingAtom();
        }),
    );
    // A page discarded in place still exists: its wake is a load of that page,
    // timed from the wake's start like a new one.
    createEffect(
        on(sl.wakeRequested, (requested) => {
            // A wake that ended without a load (woken during the park before a discard)
            // must not time a later, unrelated navigation from its start.
            if (!requested) loadClockFrom = 0;
            if (!requested || !paneCreated()) return;
            loadClockFrom = sl.wakeStartedAt();
            // A load already in flight when the wake begins (an earlier wake was abandoned before its page
            // finished) is set to true again with no change, so nothing would arm the clock and the wake
            // would never end. That load's end is this wake's end.
            sawLoading = model.loadingAtom();
        }),
    );
    createEffect(
        on(model.loadingAtom, (loading) => {
            if (!loadClockFrom) return;
            if (loading) {
                sawLoading = true;
                return;
            }
            if (!sawLoading) return;
            const ms = performance.now() - loadClockFrom;
            loadClockFrom = 0;
            if (sl.sleeping()) {
                diag(`[hibernate] wake-loaded ms=${Math.round(ms)} why=${sl.wakeWhy()} visible=${sl.visible()}`);
                if (wakeEndsNow(sl.visible(), sl.wakeWhy())) void model.endSleep(ms).catch(() => {});
                else sl.setWakeLoadedMs(ms);
            } else {
                void model.recordLoadMs(ms).catch(() => {});
            }
        }),
    );
}

/** The card shown over a shown pane while it wakes; the diagnostics note how long it was up. */
export function useSleepCard(p: { model: BrowserViewModel; sl: PaneSleep; paneCreated: Accessor<boolean>; diag: (msg: string) => void }) {
    const { model, sl, diag } = p;
    useLoadClock(p);
    const sleepCard = () => (sl.visible() && sl.wakeLoadedMs() === 0 ? model.sleepAtom() : undefined);
    let cardShownAt = 0;
    createEffect(
        on(
            () => sleepCard() != null,
            (shown) => {
                if (shown && cardShownAt) return;
                if (shown) {
                    cardShownAt = performance.now();
                    diag(`[hibernate] card-shown`);
                } else if (cardShownAt) {
                    diag(`[hibernate] card-hidden after=${Math.round(performance.now() - cardShownAt)}`);
                    cardShownAt = 0;
                }
            },
            { defer: true },
        ),
    );
    return sleepCard;
}
