// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The line in My Agents' Environment panel for the last change to an agent's model server: who pointed it
 * where (srv's own notice, `store/endpoint-notices.ts`), with an Undo while the 30-second window is open.
 */
import { createMemo, createSignal, Show, type JSX } from "solid-js";
import { Button } from "@/app/element/ui";
import { useTick } from "@/app/hook/useTick";
import { canUndoEndpoint, endpointNoticeOf, endpointNoticeText, undoEndpoint } from "@/app/store/endpoint-notices";

export const EndpointNoticeLine = (props: { agentId: string }): JSX.Element => {
    const tick = useTick(1000);
    const notice = createMemo(() => {
        const n = endpointNoticeOf(props.agentId);
        return n?.state === "open" ? n : undefined;
    });
    const undoable = createMemo(() => {
        tick();
        const n = notice();
        return n != null && canUndoEndpoint(n, Date.now());
    });
    const [error, setError] = createSignal<string | null>(null);
    const undo = async () => {
        const n = notice();
        if (!n) return;
        try {
            setError((await undoEndpoint(n.id)) ?? null);
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
    };
    return (
        <Show when={notice()}>
            {(n) => (
                <p class="agent-primitive-modal-global-note" role="status" data-testid="agent-env-endpoint-notice">
                    {endpointNoticeText(n())}
                    <Show when={undoable()}>
                        {" "}
                        <Button onClick={() => void undo()}>Undo</Button>
                    </Show>
                    <Show when={error()}>{(why) => <span class="agent-primitive-modal-error"> {why()}</span>}</Show>
                </p>
            )}
        </Show>
    );
};
