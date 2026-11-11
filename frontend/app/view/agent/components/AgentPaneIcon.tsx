// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { DualProviderLogo, dualLogoTitle } from "@/element/DualProviderLogo";
import type { JSX } from "solid-js";

import type { PaneIconInput } from "../pane-icon-input";

// The harness is the main icon and the model answering is the badge on it (DualProviderLogo, as the agent list
// draws it). noAction so the IconButton renders as a decorative icon, not a button. The button's own title names
// both, so the logo draws no tooltip of its own.
export function buildAgentPaneIcon(input: PaneIconInput): IconButtonDecl {
    return {
        elemtype: "iconbutton",
        // A function, not a node: the pane's header is drawn in more than one place (the tile preview among
        // them), and each draws its own face. One node can only sit in one of them.
        icon: (() => (
            <DualProviderLogo harness={input.harness} vendor={input.vendor} size={16} bare />
        )) as unknown as JSX.Element,
        noAction: true,
        title: dualLogoTitle(input.harness, input.vendor, input.modelText),
    };
}
