// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The mouse side buttons walk the pane selection history (pane-history.ts):
// Back re-selects the pane selected before, Forward the one after. The page
// sees them as `button` 3 and 4; over a browser pane, and as a fallback for a
// button the page never gets, the host sends a `side-button` event instead.
// Both paths share one debounce, so a press the host reports twice acts once.
// SPEC_MOUSE_SIDE_BUTTONS_PANE_AND_TAB_NAV_2026_10_19.md §5.

import { getApi } from "@/app/store/app-api";
import { settingsAtom } from "@/app/store/global";
import { modalsModel } from "@/app/store/modalmodel";
import { paneHistoryBack, paneHistoryForward } from "./pane-history";
import { decideSideButton, hostPressActs, isRepeat } from "./side-button-rules";

const SETTING = "mouse:sidebuttons";

function enabled(): boolean {
    return (settingsAtom() as Record<string, unknown> | undefined)?.[SETTING] !== false;
}

let last: { dir: -1 | 1; at: number } | null = null;

function perform(dir: -1 | 1, via: string): void {
    const now = performance.now();
    if (isRepeat(dir, now, last)) return;
    last = { dir, at: now };
    console.info(`[side-button] ${dir < 0 ? "back" : "forward"} via ${via}`);
    if (dir < 0) paneHistoryBack();
    else paneHistoryForward();
}

interface HostSideButton {
    button: "back" | "forward";
    ctrl: boolean;
    /** Shift, Alt, a Windows key or the left button was down. */
    blocked?: boolean;
    source: string;
    block_id?: string | null;
}

function onHostSideButton(p: HostSideButton): void {
    // Ctrl + button over a browser pane is handled in the host; a Ctrl press
    // reported from the window itself has nothing to do.
    if (p.ctrl) return;
    const dir = p.button === "back" ? -1 : 1;
    if (!enabled()) {
        // Off means the page keeps its own history on these buttons.
        if (p.source === "pane" && p.block_id) {
            const panes = getApi().browserPanes;
            void (dir < 0 ? panes?.goBack(p.block_id) : panes?.goForward(p.block_id));
        }
        return;
    }
    if (!hostPressActs(p, { enabled: true, modalOpen: modalsModel.hasOpenModals() })) return;
    perform(dir, `host:${p.source}`);
}

export function installSideButtons(): () => void {
    const handler = (e: MouseEvent): void => {
        const d = decideSideButton(e, { enabled: enabled(), modalOpen: modalsModel.hasOpenModals() });
        if (!d.consume) return;
        e.preventDefault();
        e.stopPropagation();
        // Press and release are consumed; the release acts, like a click. It is the
        // pointer release that acts, not the mouse one: cancelling a pointerdown makes
        // Chromium drop the mousedown and mouseup that follow it (the "compatibility
        // mouse events"), so a mouseup listener never ran and the button did nothing
        // where the host has no path of its own (Linux, macOS).
        // The mouse release still acts too, for a press made while another button is held
        // (a chord): that press has no pointerdown, and its release no pointerup until every
        // button is up, but its mousedown and mouseup are not dropped. When both arrive,
        // the debounce makes it one action.
        if ((e.type === "pointerup" || e.type === "mouseup") && d.act != null) perform(d.act, "dom");
    };
    // Capture and non-passive: the page's own handlers (a terminal's mouse
    // reporting) must not see these, and a passive listener can't cancel.
    const types = ["pointerdown", "pointerup", "mousedown", "mouseup", "auxclick"] as const;
    for (const type of types) window.addEventListener(type, handler as EventListener, { capture: true, passive: false });
    const unsub = getApi().listen<HostSideButton>("side-button", onHostSideButton);
    return () => {
        for (const type of types) window.removeEventListener(type, handler as EventListener, { capture: true });
        void unsub.then((u) => u());
    };
}
