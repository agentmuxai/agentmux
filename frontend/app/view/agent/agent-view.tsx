// Copyright 2024, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import React, { memo, useCallback, useState } from "react";
import { useAtomValue } from "jotai";
import type { AgentViewModel } from "./agent-model";
import { getProviderList, type ProviderDefinition } from "./providers";
import "./agent-view.scss";

const PROVIDER_ICONS: Record<string, string> = {
    claude: "\u2728", // sparkles
    codex: "\uD83E\uDD16", // robot
    gemini: "\uD83D\uDC8E", // gem
};

/**
 * Top-level wrapper — switches between picker and styled session view.
 */
export const AgentViewWrapper: React.FC<ViewComponentProps<AgentViewModel>> = memo(({ model }) => {
    const block = useAtomValue(model.blockAtom);
    const agentMode = block?.meta?.["agentMode"];
    const agentProvider = block?.meta?.["agentProvider"];

    if (agentMode === "styled" && agentProvider) {
        return <AgentStyledSession model={model} providerId={agentProvider} />;
    }

    return <AgentProviderPicker model={model} />;
});

AgentViewWrapper.displayName = "AgentViewWrapper";

// ── Provider button ────────────────────────────────────────────────────────────

const ProviderButton: React.FC<{
    provider: ProviderDefinition;
    mode: "raw" | "styled";
    onSelect: (providerId: string, mode: "raw" | "styled") => void;
    disabled: boolean;
}> = ({ provider, mode, onSelect, disabled }) => (
    <button
        className={`agent-provider-btn agent-provider-btn--${mode}`}
        onClick={() => onSelect(provider.id, mode)}
        disabled={disabled}
        title={mode === "styled" ? `${provider.displayName} — styled output` : `${provider.displayName} — raw terminal`}
    >
        <span className="agent-provider-icon">{PROVIDER_ICONS[provider.id] ?? "\u26A1"}</span>
        <span className="agent-provider-name">{provider.displayName}</span>
    </button>
);

// ── Picker ─────────────────────────────────────────────────────────────────────

const AgentProviderPicker: React.FC<{ model: AgentViewModel }> = memo(({ model }) => {
    const [launching, setLaunching] = useState<string | null>(null);
    const providers = getProviderList();

    const handleSelect = useCallback(
        async (providerId: string, mode: "raw" | "styled") => {
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
        },
        [model, providers]
    );

    const busy = launching !== null;

    return (
        <div className="agent-view">
            <div className="agent-document">
                <div className="agent-empty">
                    <div className="agent-connect-header">Connect</div>

                    <div className="agent-mode-group">
                        <div className="agent-mode-label">Raw</div>
                        <div className="agent-provider-buttons">
                            {providers.map((p) => (
                                <ProviderButton
                                    key={p.id}
                                    provider={p}
                                    mode="raw"
                                    onSelect={handleSelect}
                                    disabled={busy}
                                />
                            ))}
                        </div>
                    </div>

                    <div className="agent-mode-group">
                        <div className="agent-mode-label agent-mode-label--styled">Styled</div>
                        <div className="agent-provider-buttons">
                            {providers.map((p) => (
                                <ProviderButton
                                    key={p.id}
                                    provider={p}
                                    mode="styled"
                                    onSelect={handleSelect}
                                    disabled={busy}
                                />
                            ))}
                        </div>
                    </div>

                    {launching && (
                        <div className="agent-install-status">
                            Launching {launching.split("-")[0]}
                            {launching.endsWith("-styled") ? " (styled)" : ""}…
                        </div>
                    )}
                </div>
            </div>
        </div>
    );
});

AgentProviderPicker.displayName = "AgentProviderPicker";

// ── Styled session ─────────────────────────────────────────────────────────────

const AgentStyledSession: React.FC<{ model: AgentViewModel; providerId: string }> = memo(
    ({ model, providerId }) => {
        const block = useAtomValue(model.blockAtom);
        const provider = getProviderList().find((p) => p.id === providerId);
        const cliPath: string = block?.meta?.["agentCliPath"] ?? "";
        const cliArgs: string[] = block?.meta?.["agentCliArgs"] ?? [];

        const handleDisconnect = useCallback(async () => {
            const { RpcApi } = await import("@/app/store/wshclientapi");
            const { TabRpcClient } = await import("@/app/store/wshrpcutil");
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
                    },
                });
            } catch {
                // model logs internally
            }
        }, [model.blockId]);

        return (
            <div className="agent-view agent-view--styled">
                <div className="agent-styled-header">
                    <span className="agent-styled-icon">{PROVIDER_ICONS[providerId] ?? "\u26A1"}</span>
                    <span className="agent-styled-provider">{provider?.displayName ?? providerId}</span>
                    <span className="agent-styled-badge">Styled</span>
                    <button className="agent-styled-disconnect" onClick={handleDisconnect} title="Back to picker">
                        ✕
                    </button>
                </div>

                <div className="agent-document">
                    <div className="agent-styled-empty">
                        <div className="agent-styled-spinner" />
                        <div className="agent-styled-status-text">Starting session…</div>
                        <div className="agent-styled-cli-info">
                            <code>{cliPath} {cliArgs.join(" ")}</code>
                        </div>
                    </div>
                </div>
            </div>
        );
    }
);

AgentStyledSession.displayName = "AgentStyledSession";
