// Copyright 2024, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import React, { memo, useCallback, useEffect, useMemo, useState } from "react";
import { useAtomValue } from "jotai";
import type { AgentViewModel } from "./agent-model";
import { getProvider } from "./providers";
import { createAgentAtoms } from "./state";
import { useAgentStream } from "./useAgentStream";
import { AgentDocumentView } from "./components/AgentDocumentView";
import { AgentFooter } from "./components/AgentFooter";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { waveEventSubscribe } from "@/app/store/wps";
import { stringToBase64 } from "@/util/util";
import "./agent-view.scss";

// ── useForgeAgents hook ───────────────────────────────────────────────────────

function useForgeAgents(): ForgeAgent[] {
    const [agents, setAgents] = useState<ForgeAgent[]>([]);

    useEffect(() => {
        let cancelled = false;

        async function load() {
            try {
                const result = await RpcApi.ListForgeAgentsCommand(TabRpcClient);
                if (!cancelled) setAgents(result ?? []);
            } catch {
                // silently ignore
            }
        }

        load();

        const unsub = waveEventSubscribe({
            eventType: "forgeagents:changed",
            handler: () => load(),
        });

        return () => {
            cancelled = true;
            unsub();
        };
    }, []);

    return agents;
}

/**
 * Top-level wrapper — switches between agent picker and presentation view.
 */
export const AgentViewWrapper: React.FC<ViewComponentProps<AgentViewModel>> = memo(({ model }) => {
    const block = useAtomValue(model.blockAtom);
    const agentId = block?.meta?.["agentId"];

    if (agentId) {
        return <AgentPresentationView model={model} agentId={agentId} />;
    }

    return <AgentPicker model={model} />;
});

AgentViewWrapper.displayName = "AgentViewWrapper";

// ── Agent Picker ────────────────────────────────────────────────────────────────

const AgentPicker: React.FC<{ model: AgentViewModel }> = memo(({ model }) => {
    const [launching, setLaunching] = useState<string | null>(null);
    const agents = useForgeAgents();

    const handleSelect = useCallback(
        async (agent: ForgeAgent) => {
            setLaunching(agent.id);
            try {
                await model.launchForgeAgent(agent);
            } catch {
                // model logs internally
            } finally {
                setLaunching(null);
            }
        },
        [model]
    );

    const busy = launching !== null;

    if (agents.length === 0) {
        return (
            <div className="agent-view">
                <div className="agent-picker-empty">
                    <div className="agent-picker-empty-icon">✦</div>
                    <div className="agent-picker-empty-title">No agents configured</div>
                    <div className="agent-picker-empty-desc">Create an agent in the Forge to get started.</div>
                    <button className="agent-picker-forge-btn" disabled>
                        + Create an agent in the Forge
                    </button>
                </div>
            </div>
        );
    }

    return (
        <div className="agent-view">
            <div className="agent-picker">
                <div className="agent-picker-list">
                    {agents.map((agent) => (
                        <button
                            key={agent.id}
                            className={`agent-card${launching === agent.id ? " agent-card--launching" : ""}`}
                            onClick={() => handleSelect(agent)}
                            disabled={busy}
                        >
                            <span className="agent-card-icon">{agent.icon}</span>
                            <span className="agent-card-info">
                                <span className="agent-card-name">{agent.name}</span>
                                {agent.description && (
                                    <span className="agent-card-desc">{agent.description}</span>
                                )}
                            </span>
                            {launching === agent.id && <span className="agent-card-spinner" />}
                        </button>
                    ))}
                </div>
                <div className="agent-picker-footer">
                    <button className="agent-picker-forge-btn" disabled>
                        + New agent in Forge
                    </button>
                </div>
            </div>
        </div>
    );
});

AgentPicker.displayName = "AgentPicker";

// ── Presentation View ───────────────────────────────────────────────────────────

const AgentPresentationView: React.FC<{ model: AgentViewModel; agentId: string }> = memo(
    ({ model, agentId }) => {
        const block = useAtomValue(model.blockAtom);
        // agentProvider stores the provider id (claude/codex/gemini) — set when launching
        const providerKey: string = block?.meta?.["agentProvider"] ?? agentId;
        const provider = getProvider(providerKey);
        const outputFormat: string = block?.meta?.["agentOutputFormat"] ?? "claude-stream-json";

        const agentAtoms = useMemo(() => createAgentAtoms(model.blockId), [model.blockId]);

        useAgentStream({
            blockId: model.blockId,
            outputFormat,
            documentAtom: agentAtoms.documentAtom,
            streamingStateAtom: agentAtoms.streamingStateAtom,
            enabled: true,
        });

        const handleSendMessage = useCallback(
            (message: string) => {
                const b64data = stringToBase64(message + "\n");
                RpcApi.ControllerInputCommand(TabRpcClient, {
                    blockid: model.blockId,
                    inputdata64: b64data,
                }).catch(() => {
                    // logged by RPC layer
                });
            },
            [model.blockId]
        );

        const handleBack = useCallback(async () => {
            const { WOS } = await import("@/app/store/global");
            const oref = WOS.makeORef("block", model.blockId);
            try {
                await RpcApi.SetMetaCommand(TabRpcClient, {
                    oref,
                    meta: {
                        agentId: null,
                        agentProvider: null,
                        agentOutputFormat: null,
                        agentName: null,
                        agentIcon: null,
                        agentCliPath: null,
                        agentCliArgs: null,
                        agentBinDir: null,
                        controller: null,
                    },
                });
            } catch {
                // model logs internally
            }
        }, [model.blockId]);

        return (
            <div className="agent-view agent-view--presentation">
                <div className="agent-pres-header">
                    <span className="agent-pres-icon">{block?.meta?.["agentIcon"] ?? "\u26A1"}</span>
                    <span className="agent-pres-name">{block?.meta?.["agentName"] ?? provider?.displayName ?? agentId}</span>
                    <button className="agent-pres-back" onClick={handleBack} title="Back to agents">
                        ✕
                    </button>
                </div>

                <AgentDocumentView
                    documentAtom={agentAtoms.documentAtom}
                    documentStateAtom={agentAtoms.documentStateAtom}
                />

                <AgentFooter agentId={agentId} onSendMessage={handleSendMessage} />
            </div>
        );
    }
);

AgentPresentationView.displayName = "AgentPresentationView";
