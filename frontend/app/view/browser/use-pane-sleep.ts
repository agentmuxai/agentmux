// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createSignal, on, onCleanup, type Accessor } from "solid-js";
import { getApi } from "@/util/app-api";
import { getSettingsKeyAtom } from "@/store/global";
import { usePaneTabVisibility } from "@/app/block/pane-tab-visibility";
import { useWindowTabId } from "@/app/workspace/window-tab-visibility";
import type { BrowserViewModel } from "./browser-model";
import {
    awakeReason,
    hibernationEnabled,
    HOVER_WAKE_GIVE_UP_MS,
    WAKE_STUCK_MS,
    isCapturing,
    engineSupportsFreeze,
    freezeAwakeReason,
    createHandlerMemo,
    createPageActivity,
    isLaunchMount,
    LAST_CALL_META_KEY,
    lastAgentTouch,
    pageAwakeReason,
    prewakeTabId,
    createFreezeState,
    registerHibernatablePane,
    WAKE_META_KEY,
    wakeEndsNow,
    type PaneFacts,
    type SleepReason,
} from "./hibernation";

type Diag = (msg: string) => void;
type FreezeState = ReturnType<typeof createFreezeState>;
type HandlerMemo = ReturnType<typeof createHandlerMemo>;
type PageActivity = ReturnType<typeof createPageActivity>;

/** What the hibernation hooks of one pane share between them. */
interface SleepState {
    hiddenSince: number | null;
    /** The reason a pane last stayed awake, so a tick that finds the same one says nothing. */
    lastStayedAwake: string | null;
    /** Why a frozen page stays awake (something typed in it, a leave-page prompt). A frozen page
     *  runs no script, so that cannot change until it is thawed: no need to thaw it to ask again. */
    frozenKept: string | null;
}

/** What the wake machine keeps in plain variables, read by the load clock and the shown-ready check. */
interface WakeVars {
    why: string;
    /** The wake finished without a load (a page that was alive): its "loaded" time is not a measurement. */
    noLoad: boolean;
    giveUp: ReturnType<typeof setTimeout> | undefined;
    /** Ends a shown pane's wake that never finishes, whatever its load did (`WAKE_STUCK_MS`). */
    stuck: ReturnType<typeof setTimeout> | undefined;
}

/** What the window knows that keeps the pane awake (spec §5.3). */
function paneFacts(model: BrowserViewModel, activity: PageActivity): PaneFacts {
    const entries = model.actionsAtom().entries;
    return {
        url: model.urlAtom() || ((model.meta()?.["url"] as string | undefined) ?? ""),
        keepAwake: model.keepAwakeAtom(),
        driverAgent: model.driverAgentAtom() != null,
        bypass: model.bypassAtom() != null,
        attention: model.attentionAtom() != null,
        popupWindows: model.popupWindowsAtom().length > 0,
        lastAgentActionAt: lastAgentTouch(entries.length ? entries[entries.length - 1].ts : undefined, model.meta()?.[LAST_CALL_META_KEY]),
        capturing: isCapturing(model.blockId),
        activeWhileHidden: activity.active(Date.now()),
    };
}

/**
 * A pane that mounts hidden as the window first loads starts asleep, before
 * its page is ever created (spec §5.4). Its sleep record is written at
 * once; `launchAsleep` covers the moment until it arrives.
 */
function useLaunchSleep(model: BrowserViewModel, visible: Accessor<boolean>, facts: () => PaneFacts, diag: Diag): Accessor<boolean> {
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
    return sleeping;
}

/** The wake state of one pane and what starts, upgrades and abandons a wake. */
function createWake(p: { model: BrowserViewModel; visible: Accessor<boolean>; sleeping: Accessor<boolean>; diag: Diag }) {
    const { model, visible, sleeping, diag } = p;
    const [wakeRequested, setWakeRequested] = createSignal(false);
    const [wakeStartedAt, setWakeStartedAt] = createSignal(0);
    // A wake on intent (a hover or press on the tab's pill) that has loaded
    // before the pane is shown: the sleep record stays until it is, so a hover
    // with no switch puts it back to sleep instead of leaving it awake.
    const [wakeLoadedMs, setWakeLoadedMs] = createSignal(0);
    const wk: WakeVars = { why: "", noLoad: false, giveUp: undefined, stuck: undefined };
    onCleanup(() => {
        clearTimeout(wk.giveUp);
        clearTimeout(wk.stuck);
    });
    const backToSleep = () => {
        clearTimeout(wk.stuck);
        wk.noLoad = false;
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
        if (wakeEndsNow(visible(), wk.why)) {
            void model.endSleep(undefined).catch(() => {});
            return;
        }
        wk.noLoad = true;
        setWakeLoadedMs(1);
    };
    // A shown pane shows a card until its wake ends. If the load clock never sees the load end, end the
    // wake here, so the card cannot stay for good.
    const armStuck = () => {
        clearTimeout(wk.stuck);
        wk.stuck = setTimeout(() => {
            if (!sleeping() || !visible() || wakeLoadedMs() > 0) return;
            diag(`[hibernate] wake-timeout after=${WAKE_STUCK_MS}`);
            void model.endSleep(undefined).catch(() => {});
        }, WAKE_STUCK_MS);
    };
    const wake = (why: string) => {
        if (!sleeping()) return;
        if (wakeRequested()) {
            // A hover's speculative wake is already loading: an agent's call or
            // the command makes it a real one, which must not go back to sleep.
            if (wk.why === "intent" && (why === "agent" || why === "command")) {
                wk.why = why;
                clearTimeout(wk.giveUp);
                diag(`[hibernate] wake-upgraded why=${why}`);
                if (wakeLoadedMs() > 0) void model.endSleep(wk.noLoad ? undefined : wakeLoadedMs()).catch(() => {});
            }
            return;
        }
        wk.why = why;
        setWakeStartedAt(performance.now());
        setWakeRequested(true);
        diag(`[hibernate] wake-start why=${why} visible=${visible()}`);
        clearTimeout(wk.giveUp);
        armStuck();
        if (why === "intent") {
            // No switch followed: back to sleep, loaded or not.
            wk.giveUp = setTimeout(() => {
                if (sleeping() && !visible()) {
                    diag(`[hibernate] wake-abandoned loaded=${wakeLoadedMs() > 0}`);
                    backToSleep();
                }
            }, HOVER_WAKE_GIVE_UP_MS);
        }
    };
    return { wake, armStuck, wakeWithoutLoad, backToSleep, wakeRequested, wakeStartedAt, wakeLoadedMs, setWakeLoadedMs, wk };
}

/** What wakes a pane: being shown, a hover or press on its tab, an agent's call; what ends a wake. */
function useWakeTriggers(p: {
    model: BrowserViewModel;
    visible: Accessor<boolean>;
    sleeping: Accessor<boolean>;
    tabId: string;
    diag: Diag;
    w: ReturnType<typeof createWake>;
}) {
    const { model, visible, sleeping, tabId, diag } = p;
    const { wake, armStuck, backToSleep, wakeRequested, wakeLoadedMs, wk } = p.w;
    createEffect(() => {
        if (!sleeping()) {
            clearTimeout(wk.giveUp);
            backToSleep();
        }
    });
    createEffect(() => {
        if (!sleeping() || !visible()) return;
        if (!wakeRequested()) wake("show");
        // A wake asked for while hidden, now shown and not loaded: it must not outlast the bound either.
        else if (wakeLoadedMs() === 0) armStuck();
        else {
            // Loaded while hidden, now shown: awake, with no card.
            clearTimeout(wk.giveUp);
            diag(`[hibernate] wake-shown-ready ms=${Math.round(wakeLoadedMs())}`);
            void model.endSleep(wk.noLoad ? undefined : wakeLoadedMs()).catch(() => {});
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
}

/**
 * How long the pane has been out of sight, and whether its page did
 * something meanwhile (a new title or icon: a chat, an inbox).
 */
function useHiddenTracking(p: {
    model: BrowserViewModel;
    visible: Accessor<boolean>;
    sleeping: Accessor<boolean>;
    activity: PageActivity;
    handlerMemo: HandlerMemo;
    st: SleepState;
}) {
    const { model, visible, sleeping, activity, handlerMemo, st } = p;
    createEffect(
        on(visible, (v) => {
            st.hiddenSince = v ? null : Date.now();
            if (v) {
                st.lastStayedAwake = null;
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
}

interface ControlsParams {
    model: BrowserViewModel;
    visible: Accessor<boolean>;
    sleeping: Accessor<boolean>;
    facts: () => PaneFacts;
    freezer: FreezeState;
    st: SleepState;
    diag: Diag;
}

/** Thawing a frozen page, and freezing a hidden one unless something keeps it running. */
function createFreezeControls(p: ControlsParams) {
    const { model, visible, sleeping, facts, freezer, st, diag } = p;
    // A thaw the host failed leaves the page frozen: a shown pane would stay blank, so try again.
    let thawRetry: ReturnType<typeof setTimeout> | undefined;
    const thaw = (why: string, attempt = 0) => {
        if (!freezer.isFrozen()) return;
        st.frozenKept = null;
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
        st.frozenKept = null;
        if (visible()) thaw("show");
        else diag(`[hibernate] frozen`);
        return null;
    };
    return { thaw, tryFreeze };
}

/** Puts the pane to sleep unless something keeps it awake; resolves to that reason, or null when it slept. */
function createTryHibernate(p: ControlsParams & { handlerMemo: HandlerMemo; thaw: (why: string) => void }) {
    const { model, visible, sleeping, facts, freezer, st, diag, handlerMemo, thaw } = p;
    return async (reason: SleepReason): Promise<string | null> => {
        if (sleeping()) return "already asleep";
        if (visible()) return "visible";
        // What the window knows needs no page: a pane declined on it stays as it is
        // (a frozen one stays frozen).
        let declined = awakeReason(facts(), Date.now(), getSettingsKeyAtom("browser:keepawakesites")());
        if (declined == null && freezer.isFrozen() && st.frozenKept) declined = st.frozenKept;
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
                    st.frozenKept = declined;
                    if (visible()) thaw("show");
                }
            }
        }
        if (declined != null) {
            if (declined !== st.lastStayedAwake) diag(`[hibernate] stays awake reason=${JSON.stringify(declined)} asked=${reason}`);
            st.lastStayedAwake = declined;
            return declined;
        }
        st.lastStayedAwake = null;
        // The check is async: the pane may have been shown meanwhile.
        if (visible() || sleeping()) return visible() ? "visible" : "already asleep";
        // What it holds now, while it still has a page, for the card to say what sleeping freed.
        const memory = await getApi().browserPanes.paneMemory?.([model.blockId]).catch(() => undefined);
        if (visible() || sleeping()) return visible() ? "visible" : "already asleep";
        await model.hibernate(reason, memory?.[model.blockId]);
        return null;
    };
}

/**
 * Hibernation (SPEC_BROWSER_PANE_HIBERNATION_2026_10_20.md): while the
 * pane's sleep record is set it has no native page, until a wake creates
 * it again; the record clears once the page has loaded. Registers the pane
 * with the window's policy, which sleeps and freezes the hidden ones.
 */
export function usePaneSleep(model: BrowserViewModel, diag: Diag) {
    const tabId = useWindowTabId();
    const visibility = usePaneTabVisibility(model.blockId);
    const visible = () => visibility() === "active";
    const activity = createPageActivity();
    const handlerMemo = createHandlerMemo();
    const facts = () => paneFacts(model, activity);
    const st: SleepState = { hiddenSince: visible() ? null : Date.now(), lastStayedAwake: null, frozenKept: null };
    const sleeping = useLaunchSleep(model, visible, facts, diag);
    const w = createWake({ model, visible, sleeping, diag });
    useWakeTriggers({ model, visible, sleeping, tabId, diag, w });
    useHiddenTracking({ model, visible, sleeping, activity, handlerMemo, st });
    // A hidden pane awake for a while is frozen: its page runs no script and
    // no timers until it is shown again (an agent's call thaws it in the host).
    const freezer = createFreezeState((f) => getApi().browserPanes.setFrozen?.(model.blockId, f) ?? Promise.resolve(false));
    const controls = { model, visible, sleeping, facts, freezer, st, diag };
    const { thaw, tryFreeze } = createFreezeControls(controls);
    const tryHibernate = createTryHibernate({ ...controls, handlerMemo, thaw });
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
            hiddenSince: () => st.hiddenSince,
            tryHibernate,
            wake: w.wake,
            isFrozen: freezer.isFrozen,
            tryFreeze,
        }),
    );
    return {
        visible,
        sleeping,
        freezer,
        wakeWithoutLoad: w.wakeWithoutLoad,
        wakeRequested: w.wakeRequested,
        wakeStartedAt: w.wakeStartedAt,
        wakeLoadedMs: w.wakeLoadedMs,
        setWakeLoadedMs: w.setWakeLoadedMs,
        wakeWhy: () => w.wk.why,
    };
}

export type PaneSleep = ReturnType<typeof usePaneSleep>;
