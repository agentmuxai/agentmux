// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Beside the Working row's rotating slash: the kind of work the turn is waiting on, as its own animation
// and a word ("Building 4m 12s"). The slash stays the turn's sign.
// SPEC_LONG_RUNNING_ACTIVITY_ANIMATIONS_2026_11_02.md §6.

import { Show, createMemo, type JSX } from "solid-js";

import { ActivityAnimation } from "@/app/element/ActivityAnimation";
import { useTick } from "@/app/hook/useTick";
import { workingOperation, workingOperationText, type WorkingOperationInput } from "../status/working-operation";

export function WorkingRowOperation(props: { row: WorkingOperationInput }): JSX.Element {
    const tick = useTick(1000);
    const op = createMemo(() => {
        tick();
        const now = Date.now();
        const found = workingOperation(props.row, now);
        return found && { ...found, text: workingOperationText(found, now) };
    });
    return (
        <Show when={op()}>
            {(o) => (
                <span class="agent-working-row-operation">
                    <ActivityAnimation bucket={o().bucket} since={o().since} />
                    <span>{o().text}</span>
                </span>
            )}
        </Show>
    );
}
