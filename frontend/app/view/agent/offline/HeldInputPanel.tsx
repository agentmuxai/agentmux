// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Button, IconButton } from "@/app/element/ui";
import { For, Show, type JSX } from "solid-js";
import type { HeldInput, OfflineNotice } from "./offline-input";

/**
 * Messages typed while the model API was unreachable, with Send now and
 * Cancel, and what the pane did by itself once it was back (the held
 * messages it sent, the failed turn it retried or offers to).
 * docs/specs/SPEC_OFFLINE_RESILIENCE_2026_10_20.md §4.3, §4.4.
 */
export function HeldInputPanel(props: {
    held: HeldInput[];
    notices: OfflineNotice[];
    onSendNow: () => void;
    onCancel: (id: string) => void;
    onDismissNotice: (id: number) => void;
    clockTime: (ms: number) => string;
}): JSX.Element {
    return (
        <>
            <Show when={props.held.length > 0}>
                <div class="agent-pending-zone agent-held-zone" role="status">
                    <div class="agent-pending-header">
                        <span class="agent-pending-header-text">Held: will send when the API is reachable</span>
                        <Button density="compact" icon="paper-plane" onClick={() => props.onSendNow()}>
                            Send now
                        </Button>
                    </div>
                    <For each={props.held}>
                        {(msg) => (
                            <div class="agent-pending-item agent-held-item" data-held-id={msg.id}>
                                <pre>{msg.text}</pre>
                                <span class="agent-held-time">
                                    {props.clockTime(msg.heldAt)}
                                    {msg.unconfirmed ? " · may have gone already" : ""}
                                </span>
                                <Button density="compact" tone="quiet" onClick={() => props.onCancel(msg.id)}>
                                    Cancel
                                </Button>
                            </div>
                        )}
                    </For>
                </div>
            </Show>
            <For each={props.notices}>
                {(n) => (
                    <div class="agent-offline-notice" role="status">
                        <span class="agent-offline-notice-text">{n.text}</span>
                        <Show when={n.retry}>
                            {(retry) => (
                                <Button density="compact" icon="rotate-right" onClick={() => retry()()}>
                                    Retry
                                </Button>
                            )}
                        </Show>
                        <IconButton density="compact" icon="xmark" label="Dismiss" onClick={() => props.onDismissNotice(n.id)} />
                    </div>
                )}
            </For>
        </>
    );
}
