// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createSignal, For, Match, on, onCleanup, Show, Switch, type JSX } from "solid-js";
import { isAnyUnderway } from "@/app/drag/drag-session";
import { hostHas } from "@/app/host/host-caps";
import { getApi } from "@/app/store/app-api";
import { usePaneOverlay } from "@/app/platform/pane-overlay";
import { ModalLayer } from "@/element/ModalLayer";
import { useModalLayer } from "@/element/modal-layer";
import { BrainSpinner } from "@/app/element/BrainSpinner";
import { shouldReduceMotion } from "@/app/motion/motion";
import type { BrowserViewModel } from "./browser-model";
import { usePaneRectSync } from "./use-pane-rect-sync";
import { useDragSnapshot, type DragSnapshot } from "./use-drag-snapshot";
import { useFreezeFrame } from "./use-freeze-frame";
import { useBrowserAuth } from "./use-browser-auth";
import { Button } from "@/app/element/ui";
import { BrowserNavBar } from "./browser-nav-bar";
import { DownloadStrip } from "./download-strip";
import { ActionLogStrip, DrivenByStrip } from "./driven-by-strip";
import { hasTerminalPower, LATE_PROMPT_LIMIT, minutesLabel, siteLabel } from "./bypass";
import { attentionLook, dialogMessage } from "./attention-kinds";
import { usePaneTabVisibility } from "@/app/block/pane-tab-visibility";
import { useWindowTabId } from "@/app/workspace/window-tab-visibility";
import { getSettingsKeyAtom } from "@/store/global";
import { openOrFocusPaneByView } from "@/app/store/block-component-registry";
import {
    awakeReason,
    etaLabel,
    hibernationEnabled,
    HOVER_WAKE_GIVE_UP_MS,
    isCapturing,
    engineSupportsFreeze,
    freezeAwakeReason,
    createHandlerMemo,
    createPageActivity,
    isLaunchMount,
    isOverdue,
    NOT_KEPT_TEXT,
    pageAwakeReason,
    prewakeTabId,
    createFreezeState,
    registerHibernatablePane,
    sleepName,
    sleepReasonText,
    WAKE_META_KEY,
    wakeEndsNow,
    type BrowserSleep,
    type PaneFacts,
    type SleepReason,
} from "./hibernation";
import { estimatedLoadMs } from "./load-history";
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

/**
 * Shown in place of a hibernated pane's page while it wakes: what is
 * restoring, about how long it takes, and why it slept
 * (SPEC_BROWSER_PANE_HIBERNATION_2026_10_20.md §7). A `data-pane-overlay`, so
 * it stays above the native page that is loading under it.
 */
function BrowserSleepCard(props: { sleep: BrowserSleep; startedAt: () => number; onKeepAwake: () => void }): JSX.Element {
    const [now, setNow] = createSignal(performance.now());
    const tick = setInterval(() => setNow(performance.now()), 250);
    onCleanup(() => clearInterval(tick));
    // The pane's own last load, else what its site usually takes (spec §7.2).
    const expectedMs = () => props.sleep.loadMs ?? estimatedLoadMs(props.sleep.url);
    const eta = () =>
        props.startedAt() > 0 && isOverdue(expectedMs(), now() - props.startedAt())
            ? "Still loading…"
            : etaLabel(expectedMs());
    return (
        <div class="browser-sleep-card" role="status" data-pane-overlay>
            <div class="browser-sleep-card-body">
                <div class="browser-sleep-card-head">
                    <Show when={props.sleep.favicon} fallback={<i class="fa-solid fa-moon" aria-hidden="true" />}>
                        <img class="browser-sleep-card-icon" src={props.sleep.favicon} alt="" />
                    </Show>
                    <span class="browser-sleep-card-title" title={props.sleep.url}>
                        Restoring {sleepName(props.sleep)}
                    </span>
                </div>
                <div class="browser-sleep-card-eta">
                    <BrainSpinner class="browser-sleep-card-spinner" />
                    <span>{eta()}</span>
                </div>
                <p class="browser-sleep-card-why">{sleepReasonText(props.sleep.reason, props.sleep.memoryBytes)}</p>
                <Show when={!props.sleep.kept}>
                    <p class="browser-sleep-card-lost">{NOT_KEPT_TEXT}</p>
                </Show>
                <div class="browser-sleep-card-actions">
                    <Button density="compact" onClick={props.onKeepAwake}>
                        Keep this pane awake
                    </Button>
                    <Button density="compact" onClick={() => void openOrFocusPaneByView("settings")}>
                        Settings
                    </Button>
                </div>
            </div>
        </div>
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

    // Hibernation (SPEC_BROWSER_PANE_HIBERNATION_2026_10_20.md): while the
    // pane's sleep record is set it has no native page, until a wake creates
    // it again; the record clears once the page has loaded.
    const tabId = useWindowTabId();
    const visibility = usePaneTabVisibility(model.blockId);
    const visible = () => visibility() === "active";
    // What the window knows that keeps the pane awake (spec §5.3).
    const facts = (): PaneFacts => {
        const entries = model.actionsAtom().entries;
        return {
            url: model.urlAtom() || ((model.meta()?.["url"] as string | undefined) ?? ""),
            keepAwake: model.keepAwakeAtom(),
            driverAgent: model.driverAgentAtom() != null,
            bypass: model.bypassAtom() != null,
            attention: model.attentionAtom() != null,
            popupWindows: model.popupWindowsAtom().length > 0,
            lastAgentActionAt: entries.length ? entries[entries.length - 1].ts : undefined,
            capturing: isCapturing(model.blockId),
            activeWhileHidden: activity.active(Date.now()),
        };
    };
    const activity = createPageActivity();
    const handlerMemo = createHandlerMemo();
    // The reason a pane last stayed awake, so a tick that finds the same one says nothing.
    let lastStayedAwake: string | null = null;
    // A pane that mounts hidden as the window first loads starts asleep, before
    // its page is ever created (spec §5.4). Its sleep record is written at
    // once; `launchAsleep` covers the moment until it arrives.
    const [launchAsleep, setLaunchAsleep] = createSignal(
        !visible() &&
            model.sleepAtom() == null &&
            hibernationEnabled() &&
            isLaunchMount() &&
            awakeReason(facts(), Date.now(), getSettingsKeyAtom("browser:keepawakesites")()) == null,
    );
    if (launchAsleep()) {
        diag(`[hibernate] launch-asleep`);
        void model.hibernate("launch").catch(() => setLaunchAsleep(false));
    }
    const sleeping = () => model.sleepAtom() != null || launchAsleep();
    // A pane that mounts asleep has no page yet: a "kept" record is from a page
    // discarded before a restart or reload, which is gone.
    if (model.sleepAtom()?.kept) void model.clearSleepKept().catch(() => {});
    createEffect(() => {
        if (model.sleepAtom() != null && launchAsleep()) setLaunchAsleep(false);
    });
    const [wakeRequested, setWakeRequested] = createSignal(false);
    const [wakeStartedAt, setWakeStartedAt] = createSignal(0);
    // A wake on intent (a hover or press on the tab's pill) that has loaded
    // before the pane is shown: the sleep record stays until it is, so a hover
    // with no switch puts it back to sleep instead of leaving it awake.
    const [wakeLoadedMs, setWakeLoadedMs] = createSignal(0);
    let wakeWhy = "";
    // The wake finished without a load (a page that was alive): its "loaded" time is not a measurement.
    let wakeNoLoad = false;
    let wakeGiveUp: ReturnType<typeof setTimeout> | undefined;
    onCleanup(() => clearTimeout(wakeGiveUp));
    const backToSleep = () => {
        wakeNoLoad = false;
        setWakeRequested(false);
        setWakeStartedAt(0);
        setWakeLoadedMs(0);
    };
    // A wake that needs no load (the page was alive: woken during the discard park, or
    // its discard didn't happen) is done at once. It follows the policy of a loaded wake:
    // awake now if the pane is shown or the wake was asked for; otherwise ready, and back
    // to sleep if no switch follows a hover.
    const wakeWithoutLoad = () => {
        if (!sleeping()) return;
        if (wakeEndsNow(visible(), wakeWhy)) {
            void model.endSleep(undefined).catch(() => {});
            return;
        }
        wakeNoLoad = true;
        setWakeLoadedMs(1);
    };
    const wake = (why: string) => {
        if (!sleeping()) return;
        if (wakeRequested()) {
            // A hover's speculative wake is already loading: an agent's call or
            // the command makes it a real one, which must not go back to sleep.
            if (wakeWhy === "intent" && (why === "agent" || why === "command")) {
                wakeWhy = why;
                clearTimeout(wakeGiveUp);
                diag(`[hibernate] wake-upgraded why=${why}`);
                if (wakeLoadedMs() > 0) void model.endSleep(wakeNoLoad ? undefined : wakeLoadedMs()).catch(() => {});
            }
            return;
        }
        wakeWhy = why;
        setWakeStartedAt(performance.now());
        setWakeRequested(true);
        diag(`[hibernate] wake-start why=${why} visible=${visible()}`);
        clearTimeout(wakeGiveUp);
        if (why === "intent") {
            // No switch followed: back to sleep, loaded or not.
            wakeGiveUp = setTimeout(() => {
                if (sleeping() && !visible()) {
                    diag(`[hibernate] wake-abandoned loaded=${wakeLoadedMs() > 0}`);
                    backToSleep();
                }
            }, HOVER_WAKE_GIVE_UP_MS);
        }
    };
    createEffect(() => {
        if (!sleeping()) {
            clearTimeout(wakeGiveUp);
            backToSleep();
        }
    });
    createEffect(() => {
        if (!sleeping() || !visible()) return;
        if (!wakeRequested()) wake("show");
        else if (wakeLoadedMs() > 0) {
            // Loaded while hidden, now shown: awake, with no card.
            clearTimeout(wakeGiveUp);
            diag(`[hibernate] wake-shown-ready ms=${Math.round(wakeLoadedMs())}`);
            void model.endSleep(wakeNoLoad ? undefined : wakeLoadedMs()).catch(() => {});
        }
    });
    // Only a new hover or press wakes: a wake abandoned after 10 s with the
    // pointer still on the pill must not start again (and again).
    createEffect(
        on(prewakeTabId, (t) => {
            if (t != null && t === tabId && sleeping()) wake("intent");
        }),
    );
    // srv asks for a wake when an agent's Browser call reaches the pane (spec §8).
    createEffect(
        on(
            () => model.meta()?.[WAKE_META_KEY],
            (asked) => {
                if (asked != null && sleeping()) wake("agent");
            },
        ),
    );
    // How long the pane has been out of sight, and whether its page did
    // something meanwhile (a new title or icon: a chat, an inbox).
    let hiddenSince: number | null = visible() ? null : Date.now();
    createEffect(
        on(visible, (v) => {
            hiddenSince = v ? null : Date.now();
            if (v) {
                lastStayedAwake = null;
                handlerMemo.forget();
            }
        }),
    );
    createEffect(
        on([visible, sleeping, model.titleAtom, model.faviconUrlAtom, model.loadingAtom], () => {
            if (visible() || sleeping()) activity.rebase(model.titleAtom(), model.faviconUrlAtom(), model.loadingAtom(), Date.now());
            else activity.see(model.titleAtom(), model.faviconUrlAtom(), model.loadingAtom(), Date.now());
        }),
    );
    // A hidden pane awake for a while is frozen: its page runs no script and
    // no timers until it is shown again (an agent's call thaws it in the host).
    const freezer = createFreezeState((f) => getApi().browserPanes.setFrozen?.(model.blockId, f) ?? Promise.resolve(false));
    // Why a frozen page stays awake (something typed in it, a leave-page prompt). A frozen page
    // runs no script, so that cannot change until it is thawed: no need to thaw it to ask again.
    let frozenKept: string | null = null;
    const tryHibernate = async (reason: SleepReason): Promise<string | null> => {
        if (sleeping()) return "already asleep";
        if (visible()) return "visible";
        // What the window knows needs no page: a pane declined on it stays as it is
        // (a frozen one stays frozen).
        let declined = awakeReason(facts(), Date.now(), getSettingsKeyAtom("browser:keepawakesites")());
        if (declined == null && freezer.isFrozen() && frozenKept) declined = frozenKept;
        if (declined == null) {
            // Looking at the page thaws it in the host. Wait for that before the check, so the
            // flag and the host agree whatever the check answers.
            const wasFrozen = freezer.isFrozen();
            await freezer.thaw();
            if (visible() || sleeping()) return visible() ? "visible" : "already asleep";
            const mode = handlerMemo.mode(Date.now());
            const check = handlerMemo.settle(
                mode,
                await getApi().browserPanes.hibernationCheck?.(model.blockId, mode).catch(() => null),
                Date.now(),
            );
            declined = pageAwakeReason(check);
            // A reason that still allows freezing: put the page back, and remember why.
            if (declined != null && wasFrozen && freezeAwakeReason(check) == null && !visible()) {
                if (await getApi().browserPanes.setFrozen?.(model.blockId, true).catch(() => false)) {
                    freezer.markFrozen();
                    frozenKept = declined;
                    if (visible()) thaw("show");
                }
            }
        }
        if (declined != null) {
            if (declined !== lastStayedAwake) diag(`[hibernate] stays awake reason=${JSON.stringify(declined)} asked=${reason}`);
            lastStayedAwake = declined;
            return declined;
        }
        lastStayedAwake = null;
        // The check is async: the pane may have been shown meanwhile.
        if (visible() || sleeping()) return visible() ? "visible" : "already asleep";
        // What it holds now, while it still has a page, for the card to say what sleeping freed.
        const memory = await getApi().browserPanes.paneMemory?.([model.blockId]).catch(() => undefined);
        if (visible() || sleeping()) return visible() ? "visible" : "already asleep";
        await model.hibernate(reason, memory?.[model.blockId]);
        return null;
    };
    // A thaw the host failed leaves the page frozen: a shown pane would stay blank, so try again.
    let thawRetry: ReturnType<typeof setTimeout> | undefined;
    const thaw = (why: string, attempt = 0) => {
        if (!freezer.isFrozen()) return;
        frozenKept = null;
        diag(`[hibernate] thaw why=${why}`);
        void freezer.thaw().then(() => {
            if (freezer.isFrozen() && visible() && attempt < 5) thawRetry = setTimeout(() => thaw("retry", attempt + 1), 1000);
        });
    };
    onCleanup(() => clearTimeout(thawRetry));
    const tryFreeze = async (): Promise<string | null> => {
        if (freezer.isFrozen()) return "already frozen";
        if (sleeping() || visible()) return sleeping() ? "asleep" : "visible";
        if (!(await engineSupportsFreeze(() => getApi().browserPanes.discard?.(model.blockId, "is_discarded") ?? Promise.resolve(null))))
            return "this browser engine can't show a frozen page again";
        const declined =
            awakeReason(facts(), Date.now(), getSettingsKeyAtom("browser:keepawakesites")()) ??
            freezeAwakeReason(await getApi().browserPanes.hibernationCheck?.(model.blockId, "skip").catch(() => null));
        if (declined != null) return declined;
        if (visible() || sleeping()) return visible() ? "visible" : "asleep";
        if (!(await getApi().browserPanes.setFrozen?.(model.blockId, true).catch(() => false))) return "can't freeze";
        // Shown while the call was out: thaw at once.
        freezer.markFrozen();
        frozenKept = null;
        if (visible()) thaw("show");
        else diag(`[hibernate] frozen`);
        return null;
    };
    createEffect(
        on(visible, (v) => {
            if (v) thaw("show");
        }),
    );
    createEffect(
        on(prewakeTabId, (t) => {
            if (t != null && t === tabId) thaw("intent");
        }),
    );
    onCleanup(
        registerHibernatablePane({
            blockId: model.blockId,
            isVisible: visible,
            isAsleep: sleeping,
            hiddenSince: () => hiddenSince,
            tryHibernate,
            wake,
            isFrozen: freezer.isFrozen,
            tryFreeze,
        }),
    );

    // Construction ORDER matters: freeze-frame reads paneRect()/paneCreated()
    // owned by the rect-sync hook, so rect-sync must be built first.
    const rectSync = usePaneRectSync({
        model,
        placeholderRef: () => placeholderRef,
        windowLabel,
        diag,
        asleep: () => sleeping() && !wakeRequested(),
        onWakeWithoutLoad: () => wakeWithoutLoad(),
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
        unavailable: () => !visible() || sleeping() || freezer.isFrozen(),
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

    // How long a load takes from the page's creation (or the wake's start),
    // for the card's next estimate; a wake ends when it has loaded.
    let loadClockFrom = 0;
    let sawLoading = false;
    createEffect(
        on(rectSync.paneCreated, (created) => {
            if (!created) {
                loadClockFrom = 0;
                return;
            }
            loadClockFrom = sleeping() && wakeStartedAt() > 0 ? wakeStartedAt() : performance.now();
            sawLoading = model.loadingAtom();
        }),
    );
    // A page discarded in place still exists: its wake is a load of that page,
    // timed from the wake's start like a new one.
    createEffect(
        on(wakeRequested, (requested) => {
            // A wake that ended without a load (woken during the park before a discard)
            // must not time a later, unrelated navigation from its start.
            if (!requested) loadClockFrom = 0;
            if (!requested || !rectSync.paneCreated()) return;
            loadClockFrom = wakeStartedAt();
            sawLoading = false;
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
            if (sleeping()) {
                diag(`[hibernate] wake-loaded ms=${Math.round(ms)} why=${wakeWhy} visible=${visible()}`);
                if (wakeEndsNow(visible(), wakeWhy)) void model.endSleep(ms).catch(() => {});
                else setWakeLoadedMs(ms);
            } else {
                void model.recordLoadMs(ms).catch(() => {});
            }
        }),
    );
    const sleepCard = () => (visible() && wakeLoadedMs() === 0 ? model.sleepAtom() : undefined);
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
                    <div
                        class="browser-attention"
                        classList={{ "browser-attention--agent": attentionLook(a()).agent }}
                        data-kind={a().kind}
                        role="alertdialog"
                        aria-live="assertive"
                    >
                        <div class="browser-attention-head">
                            <i class={`fa-solid ${attentionLook(a()).icon}`} aria-hidden="true" />
                            <Switch
                                fallback={
                                    <span>
                                        <b>{a().agent || "An agent"}</b>{" "}
                                        {a().kind === "handoff"
                                            ? "needs you: "
                                            : a().kind === "bypass"
                                              ? "asks you to auto-approve its clicks: "
                                              : "wants to: "}
                                        {a().kind === "handoff" || a().kind === "bypass" ? a().reason : a().what}
                                    </span>
                                }
                            >
                                <Match when={attentionLook(a()).text}>{(text) => <span>{text()}</span>}</Match>
                                <Match when={a().kind === "navigation"}>
                                    <span>
                                        This pane is limited to the sites it was opened for. The page wants to{" "}
                                        {a().popup ? "open a window at " : "go to "}
                                        <b>{a().origin}</b>.
                                    </span>
                                </Match>
                                <Match when={a().kind === "dialog"}>
                                    <span>
                                        {a().dialogKind === "beforeunload" ? (
                                            <>
                                                The page at <b>{a().origin || "this site"}</b> asks whether to leave it
                                            </>
                                        ) : (
                                            <>
                                                The page at <b>{a().origin || "this site"}</b> asks
                                            </>
                                        )}
                                        {a().agent ? (
                                            <>
                                                {" "}
                                                (after <b>{a().agent}</b>'s action)
                                            </>
                                        ) : null}
                                        :
                                    </span>
                                </Match>
                            </Switch>
                        </div>
                        <Show when={a().kind === "approval" && a().description}>
                            <div class="browser-attention-target browser-attention-description">
                                {(a().description ?? "").length > 200
                                    ? `${(a().description ?? "").slice(0, 200)}…`
                                    : a().description}
                            </div>
                        </Show>
                        <Show when={a().kind === "approval" && a().why}>
                            <div class="browser-attention-target">Asked because {a().why}.</div>
                        </Show>
                        {/* The page's own words, as text in a box of their own, so
                            they can't pass for AgentMux's (dialogs spec §8.1). */}
                        <Show when={a().kind === "dialog" && dialogMessage(a().message)}>
                            <blockquote class="browser-attention-page-text">{dialogMessage(a().message)}</blockquote>
                        </Show>
                        <Show when={attentionLook(a()).showUrl && a().url}>
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
                        <Show when={a().kind === "bypass"}>
                            <div class="browser-attention-target">
                                On <b>{(a().origins ?? []).map(siteLabel).join(", ")}</b>, for{" "}
                                <b>{minutesLabel(a().minutes ?? 60)}</b>: its clicks that submit, delete or send go
                                through without asking you, and so does the page's own "Are you sure?" that follows one. It still can't type passwords or codes or run scripts, and
                                payments still ask. Stop it any time from the red strip.
                            </div>
                        </Show>
                        <Show when={a().kind === "bypass" && hasTerminalPower(a().powers)}>
                            <div class="browser-attention-target">
                                It also asks to <b>run commands in the terminal</b> on {(a().origins ?? []).map(siteLabel).join(", ")}:
                                it types real keys and presses Enter with no question per line. You can read every line in the
                                pane's Log, and Stop or Pause end it at once. Passwords and codes stay yours to type.{" "}
                                One limit: {LATE_PROMPT_LIMIT}
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
                            <Show when={attentionLook(a()).back}>
                                <Button
                                    density={attentionLook(a()).density}
                                    disabled={!model.canGoBackAtom()}
                                    onClick={() => model.resolveAttention("back").catch(() => {})}
                                >
                                    Back
                                </Button>
                            </Show>
                            <Show when={attentionLook(a()).yes}>
                                {(yes) => (
                                    <Button
                                        tone={attentionLook(a()).tone}
                                        density={attentionLook(a()).density}
                                        onClick={() => model.resolveAttention(attentionLook(a()).answer).catch(() => {})}
                                    >
                                        {yes()}
                                    </Button>
                                )}
                            </Show>
                            <Show when={a().kind === "bypass" && !a().local}>
                                <Button
                                    density={attentionLook(a()).density}
                                    title="Keep this: it is added to Settings, Agents on sites, where you can revoke it"
                                    onClick={() => model.resolveAttention("approve_always").catch(() => {})}
                                >
                                    Always for {a().agent ?? "this agent"} on this site
                                </Button>
                            </Show>
                            <Button
                                density={attentionLook(a()).density}
                                onClick={() => model.resolveAttention("cancel").catch(() => {})}
                            >
                                {attentionLook(a()).no}
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
            <Show
                when={model.driverAgentAtom()}
                fallback={
                    <Show when={model.actionsAtom().count > 0}>
                        <ActionLogStrip model={model} />
                    </Show>
                }
            >
                {(agent) => <DrivenByStrip model={model} agent={agent()} />}
            </Show>
            <DownloadStrip model={model} />
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
                <Show when={sleepCard()}>
                    {(s) => (
                        <BrowserSleepCard
                            sleep={s()}
                            startedAt={wakeStartedAt}
                            onKeepAwake={() => void model.keepAwake().catch(() => {})}
                        />
                    )}
                </Show>
                <Show when={spinnerMounted() && !sleepCard()}>
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
