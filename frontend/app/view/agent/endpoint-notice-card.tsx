// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The card in an agent's pane when its model server was changed: who pointed it where, that the person's
 * Claude sign-in is not sent to it, and an Undo for a short while. Drawn from srv's own record
 * (`store/endpoint-notices.ts`), never from an agent's words. It waits on no one, so the tone and badges stay
 * quiet; it goes by itself when the window is over.
 */
import { createMemo, createSignal, Show, type JSX } from "solid-js";
import { Button } from "@/app/element/ui";
import { useTick } from "@/app/hook/useTick";
import type { PaneRequestProps } from "@/app/store/pane-requests";
import { canUndoEndpoint, endpointNoticeText, undoEndpoint } from "@/app/store/endpoint-notices";
import type { EndpointNoticeEvent } from "@/types/rpc/EndpointNoticeEvent";
import "./endpoint-notice-card.scss";

export function EndpointNoticeCard(props: { notice: EndpointNoticeEvent } & PaneRequestProps): JSX.Element {
    const tick = useTick(1000);
    const now = createMemo(() => (tick(), Date.now()));
    const left = () => Math.max(0, Math.ceil((props.notice.expires_at - now()) / 1000));
    const [error, setError] = createSignal<string | null>(null);
    const [busy, setBusy] = createSignal(false);
    const undo = async () => {
        setBusy(true);
        try {
            const why = await undoEndpoint(props.notice.id);
            if (why) setError(why);
            else props.close();
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        } finally {
            setBusy(false);
        }
    };
    return (
        <section class="endpoint-notice-card" aria-label="A model server was changed" role="status">
            <h3 class="endpoint-notice-card-title">{endpointNoticeText(props.notice)}</h3>
            <p class="endpoint-notice-card-note">
                It applies at the agent's next launch, on a conversation of its own. Your Claude sign-in is sent only to
                Anthropic, never to that server. Recorded in the Audit.
            </p>
            <Show when={error()}>
                <div class="settings-config-error" role="alert">
                    {error()}
                </div>
            </Show>
            <div class="endpoint-notice-card-actions">
                <Button onClick={() => props.close()}>Keep it</Button>
                <Show when={canUndoEndpoint(props.notice, now())}>
                    <Button tone="attention" busy={busy()} disabled={busy()} onClick={() => void undo()}>
                        {`Undo (${left()} s)`}
                    </Button>
                </Show>
            </div>
        </section>
    );
}
