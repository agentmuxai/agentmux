// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Button } from "@/app/element/ui";
import {
    storeClosesLine,
    storeProgress,
    storeProgressLine,
    storeDownloadedLine,
    storeRestartAsk,
    storeStayed,
    storeStayedLine,
    storeTargetVersion,
} from "@/store/app-update";
import { Show, type JSX } from "solid-js";

/** What the row says while the Store downloads and has given no numbers yet. */
export const STORE_ASKING_LINE = "Asking the Microsoft Store to download the update…";

/** What the row says after Restart, until the host reports that the Store is asking or installing. */
export const STORE_PREPARING_LINE = "Getting the install ready…";

/** The lines under the Maintenance update row about "Update in the Store" and its Restart now (no window modal: the confirm is in the row). */
export function StoreUpdateNotes(props: {
    /** The version on offer ("" when the Store named none), or null when nothing is offered. */
    closesFor: string | null;
    busy: boolean;
    note: string | null;
    /** Set when the Store has the update downloaded and Restart now can install it. */
    staged: { version: string } | null;
    restartAsking: boolean;
    /** Who Restart now interrupts (the agents in the middle of a turn), once srv has said. */
    restartWho: { working: string[]; agents: number } | null;
    restarting: boolean;
    onAskRestart: () => void;
    onCancelRestart: () => void;
    onRestart: () => void;
}): JSX.Element {
    const dim = "maintenance-row-text maintenance-row-text--dim";
    return (
        <>
            <Show when={props.closesFor !== null && !props.staged}>
                <div class={dim}>{storeClosesLine(props.closesFor!)}</div>
            </Show>
            <Show when={storeStayed()}>
                <div class="maintenance-row-text maintenance-row-text--error">{storeStayedLine(storeStayed()!)}</div>
            </Show>
            <Show when={props.busy && !props.restarting}>
                <div class={dim}>{storeProgressLine(storeProgress(), storeTargetVersion()) ?? STORE_ASKING_LINE}</div>
            </Show>
            <Show when={props.staged && !props.busy}>
                <div class={dim}>{storeDownloadedLine(props.staged!.version)}</div>
                <Show when={!props.restarting && !props.restartAsking}>
                    <Button density="compact" class="maintenance-btn" onClick={props.onAskRestart}>
                        Restart now
                    </Button>
                </Show>
                <Show when={props.restartAsking && !props.restarting}>
                    <div class="maintenance-row-text maintenance-row-text--warn">{storeRestartAsk(props.restartWho?.working ?? [], props.restartWho?.agents ?? 0)}</div>
                    <Button density="compact" tone="accent" class="maintenance-btn maintenance-btn--primary" onClick={props.onRestart}>
                        Restart
                    </Button>
                    <Button density="compact" class="maintenance-btn" onClick={props.onCancelRestart}>
                        Cancel
                    </Button>
                </Show>
                <Show when={props.restarting}>
                    <div class={dim}>{storeProgressLine(storeProgress(), storeTargetVersion()) ?? STORE_PREPARING_LINE}</div>
                </Show>
            </Show>
            <Show when={props.note}>
                <div class={dim}>{props.note}</div>
            </Show>
        </>
    );
}
