// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal, Show } from "solid-js";
import { Button } from "@/app/element/ui";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

/**
 * "Update container" on the pane row that says a container agent runs an older agent image.
 * srv refuses while a turn is running and says so; the reason shows beside the button. On
 * success srv writes the "Container removed" row, which replaces this one.
 */
export function ContainerUpdateButton(props: { blockId: string }) {
    const [busy, setBusy] = createSignal(false);
    const [refused, setRefused] = createSignal<string | null>(null);
    const update = async () => {
        setBusy(true);
        setRefused(null);
        try {
            // The registry check and stopping the container can take longer than the default 5 s.
            await RpcApi.UpdateContainerCommand(TabRpcClient, { block_id: props.blockId }, { timeout: 60000 });
        } catch (e) {
            setRefused(e instanceof Error ? e.message : String(e));
        } finally {
            setBusy(false);
        }
    };
    return (
        <div class="agent-cli-notice-actions">
            <Button density="compact" busy={busy()} onClick={() => void update()}>
                {busy() ? "Updating…" : "Update container"}
            </Button>
            <Show when={refused()}>
                <span class="agent-cli-notice-refused">{refused()}</span>
            </Show>
        </div>
    );
}
