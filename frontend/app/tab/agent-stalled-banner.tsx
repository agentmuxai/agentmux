// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a pane shows when its agent view has not started after 20 s (the
 * agent-mount-watch), instead of staying blank: the agent is running, its
 * history is loading, and a Reload for the window if it never does. Gone as
 * soon as the view mounts.
 */

import { getApi } from "@/app/store/global";
import { Button } from "@/app/element/ui";
import { Show, type JSX } from "solid-js";
import { agentMountStalled } from "./agent-mount-watch";

export function AgentStalledBanner(props: { blockId: string }): JSX.Element {
    return (
        <Show when={agentMountStalled(props.blockId)}>
            <div class="agent-stalled-banner" role="status">
                <span>The agent is running; its history is loading. If this stays, reload the window.</span>
                <Button tone="accent" density="compact" icon="rotate-right" onClick={() => getApi().reloadWindow()}>
                    Reload
                </Button>
            </div>
        </Show>
    );
}
