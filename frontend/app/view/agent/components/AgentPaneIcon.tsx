// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { ProviderLogo } from "@/element/ProviderLogo";

// noAction so the IconButton renders as a decorative icon, not a button.
export function buildAgentPaneIcon(provider: string): IconButtonDecl {
    return {
        elemtype: "iconbutton",
        icon: <ProviderLogo provider={provider} size={16} />,
        noAction: true,
        title: provider,
    };
}
