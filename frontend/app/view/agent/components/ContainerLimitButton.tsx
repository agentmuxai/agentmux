// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Show, useContext } from "solid-js";
import { ModalLayerContext } from "@/app/element/modal-layer";
import { Button } from "@/app/element/ui";

/**
 * "Raise the limit" on the pane row that says a process in a container agent's container was stopped for going over
 * its memory limit, and "Container settings" on the one that says its build caches are large: both open the agent's
 * Container limits panel. Nothing is shown without a modal layer (a bare render).
 */
export function ContainerLimitButton(props: { agentId: string; agentName: string; label?: string }) {
    const modalLayer = useContext(ModalLayerContext);
    return (
        <Show when={modalLayer}>
            <div class="agent-cli-notice-actions">
                <Button
                    density="compact"
                    onClick={() =>
                        modalLayer?.open({ kind: "agent-container-limits", agentId: props.agentId, agentName: props.agentName })
                    }
                >
                    {props.label ?? "Raise the limit"}
                </Button>
            </div>
        </Show>
    );
}
