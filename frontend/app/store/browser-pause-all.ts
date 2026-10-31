// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// "Pause all agent browsing": the person's one switch for every pane an agent
// drives (docs/specs/SPEC_BROWSER_WEB_TERMINAL_TYPING_2026_10_23.md §7). srv
// keeps it (it survives a restart); this is the window's copy, kept current by
// srv's `browserpauseall` event so every window and the status bar agree.

import { createSignal } from "solid-js";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { BrowserPauseAll } from "@/types/rpc/BrowserPauseAll";

const [state, setState] = createSignal<BrowserPauseAll>({ paused: false, since_ms: null, by: null });

/** Where the switch stands. */
export const browserPauseAll = state;

/** The text the status bar uses, so nobody takes the pause for a fault: who paused it is you, or the agent that did. */
export function pausedLabel(pause: BrowserPauseAll): string {
    return `Agent browsing paused by ${pause.by ?? "you"}`;
}

let started = false;

/** Read the switch from srv and follow it. Started once at app init; the places that show it call it too, harmlessly. */
export function startBrowserPauseAll(): void {
    if (started) return;
    started = true;
    onMuxEvent(WpsEvent.BrowserPauseAll, (data) => setState(data));
    void read(5);
}

/** The first read can come before srv answers: try again a few times, so a switch left on by a restart shows. */
async function read(triesLeft: number): Promise<void> {
    try {
        setState((await RpcApi.BrowserGrantsListCommand(TabRpcClient)).pause_all);
    } catch (e) {
        console.warn("[browser-pause-all] couldn't read the switch", e);
        if (triesLeft > 1) setTimeout(() => void read(triesLeft - 1), 3000);
    }
}

/** Turn the switch on or off. Rejects if srv doesn't take it. */
export async function setAgentBrowsingPaused(paused: boolean): Promise<void> {
    setState(await RpcApi.BrowserGrantsPauseAllCommand(TabRpcClient, { paused }));
}
