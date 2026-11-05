// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { storeClosesLine, storeStayedLine, storeStayed } from "@/store/app-update";
import { Show, type JSX } from "solid-js";

/** The lines under the Maintenance update row about "Update in the Store": what it does first, what it is doing, how the last try ended. */
export function StoreUpdateNotes(props: { closesFor: string | null; busy: boolean; note: string | null }): JSX.Element {
    const dim = "maintenance-row-text maintenance-row-text--dim";
    return (
        <>
            <Show when={props.closesFor}>
                <div class={dim}>{storeClosesLine(props.closesFor!)}</div>
            </Show>
            <Show when={storeStayed()}>
                <div class="maintenance-row-text maintenance-row-text--error">{storeStayedLine(storeStayed()!)}</div>
            </Show>
            <Show when={props.busy}>
                <div class={dim}>Asking the Microsoft Store to install the update…</div>
            </Show>
            <Show when={props.note}>
                <div class={dim}>{props.note}</div>
            </Show>
        </>
    );
}
