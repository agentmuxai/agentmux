// Copyright 2024, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import React, { memo, useCallback } from "react";
import { useAtomValue } from "jotai";
import type { AgentViewModel } from "./agent-model";
import { getProviderList, type ProviderDefinition } from "./providers";
import "./agent-view.scss";

/**
 * Top-level wrapper: passes connectWithProvider into the provider picker.
 */
export const AgentViewWrapper: React.FC<ViewComponentProps<AgentViewModel>> = memo(({ model }) => {
    return <AgentProviderPicker model={model} />;
});

AgentViewWrapper.displayName = "AgentViewWrapper";

const PROVIDER_ICONS: Record<string, string> = {
    claude: "\u2728", // sparkles
    codex: "\uD83E\uDD16", // robot
    gemini: "\uD83D\uDC8E", // gem
};

const ProviderButton: React.FC<{
    provider: ProviderDefinition;
    onSelect: (providerId: string) => void;
    disabled: boolean;
}> = ({ provider, onSelect, disabled }) => {
    return (
        <button
            className="agent-provider-btn"
            onClick={() => onSelect(provider.id)}
            disabled={disabled}
        >
            <span className="agent-provider-icon">{PROVIDER_ICONS[provider.id] || "\u26A1"}</span>
            <span className="agent-provider-name">{provider.displayName}</span>
        </button>
    );
};

/**
 * Provider selection screen. Clicking a button detects/installs the CLI, then launches terminal.
 */
const AgentProviderPicker: React.FC<{ model: AgentViewModel }> = memo(({ model }) => {
    const status = useAtomValue(model.providerStatus);
    const statusMessage = useAtomValue(model.statusMessage);
    const providers = getProviderList();
    const busy = status !== "idle" && status !== "error";

    const handleProviderSelect = useCallback(
        async (providerId: string) => {
            await model.connectWithProvider(providerId);
        },
        [model]
    );

    return (
        <div className="agent-view">
            <div className="agent-document">
                <div className="agent-empty">
                    <div className="agent-connect-header">Connect</div>
                    <div className="agent-provider-buttons">
                        {providers.map((provider) => (
                            <ProviderButton
                                key={provider.id}
                                provider={provider}
                                onSelect={handleProviderSelect}
                                disabled={busy}
                            />
                        ))}
                    </div>
                    {statusMessage && (
                        <div className={`agent-install-status ${status === "error" ? "agent-install-error" : ""}`}>
                            {statusMessage}
                        </div>
                    )}
                </div>
            </div>
        </div>
    );
});

AgentProviderPicker.displayName = "AgentProviderPicker";
