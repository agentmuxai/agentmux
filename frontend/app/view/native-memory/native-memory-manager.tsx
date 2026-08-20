// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * NativeMemoryManager — Armory's "Native Memory" rail tab: an agent picker
 * in front of the same NativeMemoryHistoryPanel a Stash pane's own Memory
 * tab uses. Both read the identical agent:memory:{list,history,diff,revert}
 * RPCs — one source of truth, two entry points (per-agent Stash, cross-
 * agent Armory) — no copy or migration step between them. See
 * docs/specs/SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md §4.3.
 *
 * Deliberately named/placed away from `frontend/app/view/memory/` (Armory's
 * *Bundles* tab — `MemoryManager`, a legacy name predating the
 * Preset→Bundle rename — see that spec's §4.3 "naming collision to avoid").
 * This directory and component are about native memory only.
 */

import { createEffect, createSignal, For, Show, type JSX } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { useAgentDefinitions } from "@/app/view/agent/components/AgentPicker";
import { NativeMemoryHistoryPanel } from "@/app/view/agent/components/NativeMemoryHistoryPanel";
import "./native-memory-manager.scss";

export function NativeMemoryManager(): JSX.Element {
    const [agents] = useAgentDefinitions();
    const [selectedAgentId, setSelectedAgentId] = createSignal<string>("");
    const [files, setFiles] = createSignal<NativeMemoryFileMeta[]>([]);
    const [selectedFilename, setSelectedFilename] = createSignal<string>("");
    const [filesLoading, setFilesLoading] = createSignal(false);
    const [filesError, setFilesError] = createSignal<string | null>(null);

    // Re-fetch the file list whenever the selected agent changes; clear the
    // file selection so a stale filename from a different agent can't leak
    // into NativeMemoryHistoryPanel's props.
    createEffect(() => {
        const agentId = selectedAgentId();
        setSelectedFilename("");
        setFiles([]);
        if (!agentId) return;
        setFilesLoading(true);
        setFilesError(null);
        RpcApi.NativeMemoryListCommand(TabRpcClient, { agent_id: agentId })
            .then((res) => setFiles(res.files))
            .catch((e: Error) => setFilesError(`Failed to list memory files: ${e.message ?? e}`))
            .finally(() => setFilesLoading(false));
    });

    return (
        <div class="native-memory-manager">
            <div class="native-memory-manager-header">
                <label class="native-memory-manager-field">
                    <span>Agent</span>
                    <select
                        value={selectedAgentId()}
                        onChange={(e) => setSelectedAgentId(e.currentTarget.value)}
                    >
                        <option value="">Select an agent…</option>
                        <For each={agents()}>
                            {(agent) => <option value={agent.id}>{agent.name || agent.slug || agent.id}</option>}
                        </For>
                    </select>
                </label>

                <Show when={selectedAgentId()}>
                    <label class="native-memory-manager-field">
                        <span>File</span>
                        <select
                            value={selectedFilename()}
                            disabled={filesLoading() || files().length === 0}
                            onChange={(e) => setSelectedFilename(e.currentTarget.value)}
                        >
                            <option value="">
                                {filesLoading() ? "Loading…" : files().length === 0 ? "No memory files" : "Select a file…"}
                            </option>
                            <For each={files()}>
                                {(file) => <option value={file.filename}>{file.filename}</option>}
                            </For>
                        </select>
                    </label>
                </Show>
            </div>

            <Show when={filesError()}>
                <div class="native-memory-manager-error">{filesError()}</div>
            </Show>

            <div class="native-memory-manager-body">
                <Show
                    when={selectedAgentId() && selectedFilename()}
                    fallback={
                        <div class="native-memory-manager-empty">
                            {selectedAgentId()
                                ? "Pick a memory file to see its version history."
                                : "Pick an agent to browse its native memory version history."}
                        </div>
                    }
                >
                    {/* Keyed on agentId:filename so switching either forces a
                        clean remount — NativeMemoryHistoryPanel's own doc
                        comment: it does not react to prop changes after
                        mount by design. */}
                    <Show when={`${selectedAgentId()}:${selectedFilename()}`} keyed>
                        {(_key) => (
                            <NativeMemoryHistoryPanel
                                agentId={selectedAgentId()}
                                filename={selectedFilename()}
                            />
                        )}
                    </Show>
                </Show>
            </div>
        </div>
    );
}

NativeMemoryManager.displayName = "NativeMemoryManager";
