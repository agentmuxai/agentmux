// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal, onCleanup, Show, type Accessor, type JSX } from "solid-js";
import { isAnyUnderway } from "@/app/drag/drag-session";
import { hostHas } from "@/util/host-caps";
import { getApi } from "@/util/app-api";
import { usePaneOverlay } from "@/app/platform/pane-overlay";
import { useModalLayer } from "@/element/modal-layer";
import { BrainSpinner } from "@/app/element/BrainSpinner";
import type { BrowserViewModel } from "./browser-model";
import { usePaneRectSync } from "./use-pane-rect-sync";
import { useDragSnapshot, type DragSnapshot } from "./use-drag-snapshot";
import { useFreezeFrame } from "./use-freeze-frame";
import { usePaneSleep } from "./use-pane-sleep";
import { useLoadingOverlay, useSleepCard, type LoadingOverlay } from "./use-load-overlay";
import { useBrowserAuth } from "./use-browser-auth";
import { Button } from "@/app/element/ui";
import { BrowserSigninOffer, BrowserSigninPending } from "./browser-signin-offer";
import { BrowserAttentionBanner } from "./browser-attention";
import { BrowserNavBar } from "./browser-nav-bar";
import { BrowserPopupNotes } from "./browser-popups";
import { DownloadStrip } from "./download-strip";
import { DrivenByStrip } from "./driven-by-strip";
import { NoDriverStrips } from "./give-back-strip";
import { openOrFocusPaneByView } from "@/app/store/block-component-registry";
import { etaLabel, isOverdue, NOT_KEPT_TEXT, sleepName, sleepReasonText, type BrowserSleep } from "./hibernation";
import { estimatedLoadMs } from "./load-history";
import "./browser-view.scss";
import { tip } from "@/app/element/tip";

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
 * The browser pane. Its `useModalLayer()` (the HTTP Basic / Digest auth modal on a 401-protected URL) resolves
 * to the block frame's pane layer, so that modal locks only this pane: everything else in the tab (sibling
 * panes, tab bar, title bar) stays interactive.
 * SPEC_LAUNCH_MODAL_PANE_SCOPE_2026_05_25.md §5 (browser-auth follow-up),
 * SPEC_PANE_MODALS_NOT_WINDOW_MODALS_2026_11_02.md D3.
 */
export function BrowserViewComponent(props: { model: BrowserViewModel }): JSX.Element {
    // A browser pane is a native browser drawn by the host. A host without
    // them (HostCaps.nativeBrowserPane) gets a notice, never a pane that
    // issues commands nothing will answer.
    return (
        <Show when={hostHas("nativeBrowserPane")} fallback={<BrowserPaneUnavailable />}>
            <BrowserViewInner model={props.model} />
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
                    <span class="browser-sleep-card-title" {...tip(props.sleep.url)}>
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

/** The person pressed inside the pane. */
function focusPageOnPress(model: BrowserViewModel, paneCreated: Accessor<boolean>): void {
    // An explicit click hands the pane HWND keyboard focus. Not on hover: the address bar
    // sits right above the page, so the cursor drifting back over it re-grabbed focus
    // before the user could type.
    if (paneCreated() && !model.closed) {
        getApi().browserPanes.focus(model.blockId).catch(() => {});
    }
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

    const sl = usePaneSleep(model, diag);
    // Construction ORDER matters: freeze-frame reads paneRect()/paneCreated()
    // owned by the rect-sync hook, so rect-sync must be built first.
    const rectSync = usePaneRectSync({
        model,
        placeholderRef: () => placeholderRef,
        windowLabel,
        diag,
        asleep: () => sl.sleeping() && !sl.wakeRequested(),
        onWakeWithoutLoad: () => sl.wakeWithoutLoad(),
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
        unavailable: () => !sl.visible() || sl.sleeping() || sl.freezer.isFrozen(),
        diag,
    });
    useBrowserAuth({ model, modalLayer, diag });
    const loadingOverlay = useLoadingOverlay(model);
    const sleepCard = useSleepCard({ model, sl, paneCreated: rectSync.paneCreated, diag });

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
                {(a) => <BrowserAttentionBanner model={model} attention={a} />}
            </Show>
            <BrowserSigninPending blockId={model.blockId} meta={() => model.meta()} />
            <BrowserSigninOffer blockId={model.blockId} meta={() => model.meta()} covered={() => model.attentionAtom() != null} />
            <BrowserPopupNotes model={model} />
            <Show
                when={model.driverAgentAtom()}
                fallback={<NoDriverStrips model={model} />}
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
                onMouseDown={() => focusPageOnPress(model, rectSync.paneCreated)}
            >
                <BrowserPageOverlays
                    model={model}
                    paneCreated={rectSync.paneCreated}
                    freeze={freeze}
                    sleepCard={sleepCard}
                    wakeStartedAt={sl.wakeStartedAt}
                    loadingOverlay={loadingOverlay}
                    takeSnapshot={dragSnapshot.take}
                />
            </div>
        </div>
    );
}

/** What is drawn inside the page's placeholder: the empty state, the freeze snapshot, the sleep card, the loading overlay and the drag catcher. */
function BrowserPageOverlays(props: {
    model: BrowserViewModel;
    paneCreated: Accessor<boolean>;
    freeze: ReturnType<typeof useFreezeFrame>;
    sleepCard: Accessor<BrowserSleep | undefined>;
    wakeStartedAt: Accessor<number>;
    loadingOverlay: LoadingOverlay;
    takeSnapshot: () => Promise<DragSnapshot | null>;
}): JSX.Element {
    const { model, freeze, sleepCard, loadingOverlay } = props;
    return (
        <>
            <Show when={!model.urlAtom() && !props.paneCreated()}>
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

            {/* `data-pane-overlay`: pane-overlay-auto.ts finds the element and punches a hole through the
                native browser-pane HWND, which paints above the DOM whatever the z-index
                (SPEC_PANE_OVERLAY_AUTO_CLIP_2026_05_11.md). The fade-out opacity goes on THIS wrapper
                (is-fading), not only on BrainSpinner: the clip reads the tagged element's computed
                opacity, so the hole lifts as it fades. Layer 2
                (SPEC_BROWSER_PANE_LOADING_INDICATOR_FLICKER_2026_08_17.md): full-pane coverage only
                for the first load; once the pane has painted, a later load gets a small corner badge. */}
            <Show when={sleepCard()}>
                {(s) => (
                    <BrowserSleepCard
                        sleep={s()}
                        startedAt={props.wakeStartedAt}
                        onKeepAwake={() => void model.keepAwake().catch(() => {})}
                    />
                )}
            </Show>
            <Show when={loadingOverlay.spinnerMounted() && !sleepCard()}>
                <Show
                    when={!loadingOverlay.hasPaintedOnce()}
                    fallback={
                        <div
                            class="browser-loading-badge"
                            classList={{ "is-fading": loadingOverlay.spinnerFading() }}
                            data-pane-overlay
                        >
                            <BrainSpinner class="browser-loading-badge-spinner" />
                        </div>
                    }
                >
                    <div class="browser-loading-overlay" classList={{ "is-fading": loadingOverlay.spinnerFading() }} data-pane-overlay>
                        <BrainSpinner />
                    </div>
                </Show>
            </Show>
            <Show when={isAnyUnderway(ELEMENT_DRAG_KINDS)}>
                <BrowserDragCatcher takeSnapshot={props.takeSnapshot} />
            </Show>
        </>
    );
}
