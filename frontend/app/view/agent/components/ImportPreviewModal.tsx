// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createMemo, createSignal, For, Show, type JSX } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { Button } from "@/element/button";
import { Modal, ModalBody, ModalFooter, ModalHeader } from "@/element/modal-v2";

interface ImportPreviewModalProps {
    payload: ExportForgeAgentsResult | null;
    onClose: () => void;
}

export const ImportPreviewModal = (props: ImportPreviewModalProps): JSX.Element => {
    const [existingSlugs, setExistingSlugs] = createSignal<Set<string>>(new Set());
    const [importing, setImporting] = createSignal(false);
    const [resultMsg, setResultMsg] = createSignal<string | null>(null);

    createEffect(async () => {
        if (props.payload == null) return;
        try {
            const agents = await RpcApi.ListForgeAgentsCommand(TabRpcClient);
            setExistingSlugs(new Set((agents ?? []).map((a) => a.slug)));
        } catch {
            // proceed without dedup info
        }
    });

    const agentRows = createMemo(() => {
        if (!props.payload?.agents) return [];
        return props.payload.agents.map((a) => ({
            agent: a,
            skip: existingSlugs().has(a.id),
        }));
    });

    const toImportCount = createMemo(() => agentRows().filter((r) => !r.skip).length);
    const skipCount = createMemo(() => agentRows().filter((r) => r.skip).length);

    async function handleConfirm() {
        if (importing() || toImportCount() === 0) return;
        setImporting(true);
        try {
            const result = await RpcApi.ImportForgeAgentsCommand(TabRpcClient, {
                agents: (props.payload?.agents ?? []).map((a) => ({
                    id: a.id,
                    name: a.name,
                    icon: a.icon,
                    description: a.description,
                    provider: a.provider,
                    shell: a.shell,
                    working_directory: a.working_directory,
                    agent_bus_id: a.agent_bus_id,
                    agent_type: a.agent_type,
                    environment: a.environment,
                    restart_on_crash: a.restart_on_crash,
                    content: a.content,
                    skills: a.skills,
                })),
            });
            const parts: string[] = [];
            if (result.imported.length > 0) parts.push(`Imported ${result.imported.length} agent${result.imported.length !== 1 ? "s" : ""}`);
            if (result.skipped.length > 0) parts.push(`${result.skipped.length} skipped`);
            if (result.failed.length > 0) parts.push(`${result.failed.length} failed`);
            setResultMsg(parts.join(" · "));
            setTimeout(() => {
                props.onClose();
            }, 1500);
        } catch (err) {
            setResultMsg(`Import failed: ${err}`);
        } finally {
            setImporting(false);
        }
    }

    return (
        <Show when={props.payload != null}>
            <Modal
                open={true}
                onClose={() => { if (!importing()) props.onClose(); }}
                closeOnBackdropClick={!importing()}
                closeOnEscape={!importing()}
                size="md"
            >
                <ModalHeader title="Import Agents" />
                <ModalBody>
                    <Show
                        when={resultMsg() == null}
                        fallback={<div class="agent-import-result">{resultMsg()}</div>}
                    >
                        <div class="agent-import-count">
                            Found {agentRows().length} agent{agentRows().length !== 1 ? "s" : ""} in file:
                        </div>
                        <div class="agent-import-list">
                            <For each={agentRows()}>
                                {(row) => (
                                    <div classList={{
                                        "agent-import-row": true,
                                        "agent-import-row-skip": row.skip,
                                    }}>
                                        <span class="agent-import-row-status">
                                            {row.skip ? "⏭" : "✓"}
                                        </span>
                                        <span class="agent-import-row-icon">{row.agent.icon}</span>
                                        <span class="agent-import-row-name">{row.agent.name}</span>
                                        <Show when={row.agent.description}>
                                            <span class="agent-import-row-desc"> — {row.agent.description}</span>
                                        </Show>
                                        <Show when={row.skip}>
                                            <span class="agent-import-row-skip-label"> (already exists)</span>
                                        </Show>
                                    </div>
                                )}
                            </For>
                        </div>
                        <div class="agent-import-summary">
                            {toImportCount()} will be imported
                            <Show when={skipCount() > 0}>
                                {" "}&middot; {skipCount()} will be skipped
                            </Show>
                        </div>
                    </Show>
                </ModalBody>
                <ModalFooter>
                    <Button onClick={props.onClose} disabled={importing()}>
                        Cancel
                    </Button>
                    <Button
                        onClick={handleConfirm}
                        disabled={importing() || toImportCount() === 0}
                    >
                        {importing() ? "Importing…" : `Import ${toImportCount()} Agent${toImportCount() !== 1 ? "s" : ""}`}
                    </Button>
                </ModalFooter>
            </Modal>
        </Show>
    );
};

ImportPreviewModal.displayName = "ImportPreviewModal";
