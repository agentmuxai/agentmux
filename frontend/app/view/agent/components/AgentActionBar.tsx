// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal, type JSX } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { ImportPreviewModal } from "./ImportPreviewModal";

export const AgentActionBar = (): JSX.Element => {
    const [importing, setImporting] = createSignal(false);
    const [exporting, setExporting] = createSignal(false);
    const [importPayload, setImportPayload] = createSignal<ExportForgeAgentsResult | null>(null);
    const [addingAgent, setAddingAgent] = createSignal(false);

    async function handleAddAgent() {
        if (addingAgent()) return;
        setAddingAgent(true);
        try {
            await RpcApi.CreateForgeAgentCommand(TabRpcClient, {
                name: "New Agent",
                provider: "claude",
                icon: "✦",
                description: "",
            });
        } catch (err) {
            console.error("AgentActionBar: failed to create agent", err);
        } finally {
            setAddingAgent(false);
        }
    }

    function handleImportClick() {
        if (importing()) return;
        const input = document.createElement("input");
        input.type = "file";
        input.accept = ".json";
        input.style.display = "none";
        input.onchange = async () => {
            const file = input.files?.[0];
            if (!file) return;
            setImporting(true);
            try {
                const text = await file.text();
                let parsed: ExportForgeAgentsResult;
                try {
                    parsed = JSON.parse(text);
                } catch {
                    alert("Invalid file — expected AgentMux export format (JSON parse error)");
                    return;
                }
                if (!parsed?.agents || !Array.isArray(parsed.agents)) {
                    alert("Invalid file — expected AgentMux export format (missing agents array)");
                    return;
                }
                setImportPayload(parsed);
            } catch (err) {
                console.error("AgentActionBar: import read error", err);
                alert("Failed to read file");
            } finally {
                setImporting(false);
            }
        };
        document.body.appendChild(input);
        input.click();
        document.body.removeChild(input);
    }

    async function handleExport() {
        if (exporting()) return;
        setExporting(true);
        try {
            const result = await RpcApi.ExportForgeAgentsCommand(TabRpcClient);
            const json = JSON.stringify(result, null, 2);
            const blob = new Blob([json], { type: "application/json" });
            const url = URL.createObjectURL(blob);
            const today = new Date().toISOString().slice(0, 10);
            const a = document.createElement("a");
            a.href = url;
            a.download = `agentmux-agents-${today}.json`;
            document.body.appendChild(a);
            a.click();
            document.body.removeChild(a);
            URL.revokeObjectURL(url);
        } catch (err) {
            console.error("AgentActionBar: export error", err);
            alert("Export failed");
        } finally {
            setExporting(false);
        }
    }

    return (
        <>
            <div class="agent-action-bar">
                <button
                    class="agent-action-btn"
                    classList={{ "agent-action-btn-disabled": addingAgent() }}
                    onClick={handleAddAgent}
                    title="Create a new forge agent"
                >
                    + Add Agent
                </button>
                <button
                    class="agent-action-btn"
                    classList={{ "agent-action-btn-disabled": importing() }}
                    onClick={handleImportClick}
                    title="Import agents from a JSON file"
                >
                    ↓ Import
                </button>
                <button
                    class="agent-action-btn"
                    classList={{ "agent-action-btn-disabled": exporting() }}
                    onClick={handleExport}
                    title="Export all agents to a JSON file"
                >
                    ↑ Export
                </button>
            </div>
            <ImportPreviewModal
                payload={importPayload()}
                onClose={() => setImportPayload(null)}
            />
        </>
    );
};

AgentActionBar.displayName = "AgentActionBar";
