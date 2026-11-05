// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** McpManager's delete confirmation, and what it says deleting a catalog server takes away. */

import { Show, type JSX } from "solid-js";

import { ConfirmModal } from "@/app/element/confirm-modal";
import type { McpServerCatalogItem } from "@/types/rpc/McpServerCatalogItem";
import { reachLabel } from "./mcp-model";
import type { McpState } from "./mcp-manager-state";

/** What deleting a catalog server takes away, and from whom. */
function DeleteDetails(props: {
    server: McpServerCatalogItem;
    /** Null while they're being looked up. */
    boundNames: string[] | null;
    runningIn: string[];
}): JSX.Element {
    const bound = () => {
        const names = props.boundNames;
        if (names && names.length > 0) return `Bound to ${names.join(", ")}.`;
        if (props.server.bound_count > 0) return `Bound to ${reachLabel({ ...props.server, all_agents: false })}.`;
        return null;
    };
    const running = () => [...new Set(props.runningIn)];
    return (
        <div class="mcp-delete-confirm">
            <p>AgentMux removes it from the catalog. Agents stop getting it the next time they start.</p>
            <Show when={props.server.all_agents}>
                <p>Every agent gets it now.</p>
            </Show>
            <Show when={bound()}>{(text) => <p>{text()}</p>}</Show>
            <Show when={running().length > 0}>
                <p>Running now in {running().join(", ")}, which keep it until they restart.</p>
            </Show>
        </div>
    );
}

export function McpDeleteConfirm(props: { s: McpState }): JSX.Element {
    const s = props.s;
    return (
        <Show when={s.deleting()}>
            {(server) => (
                <ConfirmModal
                    open={true}
                    scope="pane"
                    title={`Delete ${server().name}?`}
                    confirmLabel="Delete server"
                    destructive
                    onConfirm={async () => {
                        await s.model.deleteServer(server().id);
                        s.setDeleting(null);
                    }}
                    onCancel={() => s.setDeleting(null)}
                >
                    <DeleteDetails
                        server={server()}
                        boundNames={s.deletingBound()}
                        runningIn={s.usesFor(server().name).map((u) => u.agent)}
                    />
                </ConfirmModal>
            )}
        </Show>
    );
}
