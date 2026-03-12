// Copyright 2024, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { createMemo, createSignal, Show, type JSX } from "solid-js";
import type { AgentViewModel } from "./agent-model";
import { getProviderList, type ProviderDefinition } from "./providers";
import { createAgentAtoms } from "./state";
import { useAgentStream } from "./useAgentStream";
import { AgentDocumentView } from "./components/AgentDocumentView";
import { AgentFooter } from "./components/AgentFooter";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { stringToBase64 } from "@/util/util";
import "./agent-view.scss";

const PROVIDER_ICONS: Record<string, string> = {
    claude: "\u2728", // sparkles
    codex: "\uD83E\uDD16", // robot
    gemini: "\uD83D\uDC8E", // gem
};

/**
 * Top-level wrapper — switches between picker and styled session view.
 */
export const AgentViewWrapper = ({ model }: { model: AgentViewModel }): JSX.Element => {
    const block = model.blockAtom;
    const agentMode = () => block()?.meta?.["agentMode"];
    const agentProvider = () => block()?.meta?.["agentProvider"];

    return (
        <Show
            when={agentMode() === "styled" && agentProvider()}
            fallback={<AgentProviderPicker model={model} />}
        >
            <AgentStyledSession model={model} providerId={agentProvider()} />
        </Show>
    );
};

AgentViewWrapper.displayName = "AgentViewWrapper";

// ── Provider button ────────────────────────────────────────────────────────────

const ProviderButton = ({
    provider,
    mode,
    onSelect,
    disabled,
}: {
    provider: ProviderDefinition;
    mode: "raw" | "styled";
    onSelect: (providerId: string, mode: "raw" | "styled") => void;
    disabled: boolean;
}): JSX.Element => (
    <button
        class={`agent-provider-btn agent-provider-btn--${mode}`}
        onClick={() => onSelect(provider.id, mode)}
        disabled={disabled}
        title={mode === "styled" ? `${provider.displayName} — styled output` : `${provider.displayName} — raw terminal`}
    >
        <span class="agent-provider-icon">{PROVIDER_ICONS[provider.id] ?? "\u26A1"}</span>
        <span class="agent-provider-name">{provider.displayName}</span>
    </button>
);

// ── Picker ─────────────────────────────────────────────────────────────────────

const AgentProviderPicker = ({ model }: { model: AgentViewModel }): JSX.Element => {
    const [launching, setLaunching] = createSignal<string | null>(null);
    const providers = getProviderList();

    const handleSelect = async (providerId: string, mode: "raw" | "styled") => {
        const provider = providers.find((p) => p.id === providerId);
        if (!provider) return;
        setLaunching(`${providerId}-${mode}`);
        try {
            if (mode === "raw") {
                await model.connectWithProvider(providerId, provider.cliCommand);
            } else {
                await model.connectStyled(providerId, provider.cliCommand);
            }
        } catch {
            // model logs internally
        } finally {
            setLaunching(null);
        }
    };

    const busy = () => launching() !== null;

    return (
        <div class="agent-view">
            <div class="agent-document">
                <div class="agent-empty">
                    <div class="agent-connect-header">Connect</div>

                    <div class="agent-mode-group">
                        <div class="agent-mode-label">Raw</div>
                        <div class="agent-provider-buttons">
                            {providers.map((p) => (
                                <ProviderButton
                                    provider={p}
                                    mode="raw"
                                    onSelect={handleSelect}
                                    disabled={busy()}
                                />
                            ))}
                        </div>
                    </div>

                    <div class="agent-mode-group">
                        <div class="agent-mode-label agent-mode-label--styled">Styled</div>
                        <div class="agent-provider-buttons">
                            {providers.map((p) => (
                                <ProviderButton
                                    provider={p}
                                    mode="styled"
                                    onSelect={handleSelect}
                                    disabled={busy()}
                                />
                            ))}
                        </div>
                    </div>

                    <Show when={launching()}>
                        <div class="agent-install-status">
                            Launching {launching().split("-")[0]}
                            {launching().endsWith("-styled") ? " (styled)" : ""}…
                        </div>
                    </Show>
                </div>
            </div>
        </div>
    );
};

AgentProviderPicker.displayName = "AgentProviderPicker";

// ── Styled session ─────────────────────────────────────────────────────────────

const AgentStyledSession = ({ model, providerId }: { model: AgentViewModel; providerId: string }): JSX.Element => {
    const block = model.blockAtom;
    const provider = getProviderList().find((p) => p.id === providerId);
    const outputFormat = () => block()?.meta?.["agentOutputFormat"] ?? "claude-stream-json";

    // Create per-instance signals (stable across re-renders via createMemo keyed on blockId)
    const agentAtoms = createMemo(() => createAgentAtoms(model.blockId));

    // Subscribe to PTY output and parse into DocumentNodes
    useAgentStream({
        blockId: model.blockId,
        outputFormat: outputFormat(),
        documentAtom: agentAtoms().documentAtom,
        streamingStateAtom: agentAtoms().streamingStateAtom,
        enabled: true,
    });

    // Send user message to the PTY via ControllerInputCommand
    const handleSendMessage = (message: string) => {
        const b64data = stringToBase64(message + "\n");
        RpcApi.ControllerInputCommand(TabRpcClient, {
            blockid: model.blockId,
            inputdata64: b64data,
        }).catch(() => {
            // logged by RPC layer
        });
    };

    const handleDisconnect = async () => {
        const { WOS } = await import("@/app/store/global");
        const oref = WOS.makeORef("block", model.blockId);
        try {
            await RpcApi.SetMetaCommand(TabRpcClient, {
                oref,
                meta: {
                    agentMode: null,
                    agentProvider: null,
                    agentCliPath: null,
                    agentCliArgs: null,
                    agentOutputFormat: null,
                    agentBinDir: null,
                    controller: null,
                },
            });
        } catch {
            // model logs internally
        }
    };

    return (
        <div class="agent-view agent-view--styled">
            <div class="agent-styled-header">
                <span class="agent-styled-icon">{PROVIDER_ICONS[providerId] ?? "\u26A1"}</span>
                <span class="agent-styled-provider">{provider?.displayName ?? providerId}</span>
                <span class="agent-styled-badge">Styled</span>
                <button class="agent-styled-disconnect" onClick={handleDisconnect} title="Back to picker">
                    ✕
                </button>
            </div>

            <AgentDocumentView
                documentAtom={agentAtoms().documentAtom}
                documentStateAtom={agentAtoms().documentStateAtom}
            />

            <AgentFooter agentId={providerId} onSendMessage={handleSendMessage} />
        </div>
    );
};

AgentStyledSession.displayName = "AgentStyledSession";
