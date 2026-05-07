// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { ProviderLogo } from "@/element/ProviderLogo";

/**
 * Builds the IconButtonDecl rendered in the agent pane's frame header.
 * When the block has an `agentProvider` meta key, the icon becomes the
 * provider's brand logo so users can tell Claude / Codex / Gemini panes
 * apart at a glance (issue #680).
 */
export function buildAgentPaneIcon(provider: string): IconButtonDecl {
    return {
        elemtype: "iconbutton",
        icon: <ProviderLogo provider={provider} size={16} />,
        noAction: true,
        title: provider,
    };
}
