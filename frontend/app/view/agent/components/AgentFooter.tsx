// Copyright 2024, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentFooter - Minimal Claude Code-style input
 *
 * The composer's behaviour is in composer-actions.ts (writes, input, send, undo) and
 * composer-keys.ts (key handling), over the state in composer-context.ts. Its draft is kept per
 * block outside this component (reload-persistence.ts), so it survives this component's own
 * unmount, which happens on every ordinary pane-stack tab switch; the textarea is UNCONTROLLED
 * (the DOM owns the value, for typing performance), so nothing else would hold a copy. The draft is
 * cleared only on send, never on cleanup.
 * Spec: SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md §3.4.
 */

import { Show, type JSX } from "solid-js";
import { FooterMic, FooterOverlays, FooterTextarea } from "./agent-footer-parts";
import { createAgentFooter, type AgentFooterProps } from "./agent-footer-state";

export const AgentFooter = (props: AgentFooterProps): JSX.Element => {
    const s = createAgentFooter(props);

    return (
        <div class="agent-footer">
            <div class="agent-input-container">
                <FooterOverlays s={s} />
                <FooterTextarea s={s} />
                {/* Pinned to the composer's right edge instead of the pane's header (moved off
                    blockframe.tsx, SPEC_AGENT_WORKING_INDICATOR_SHIMMER_AND_MIC_RELOCATION_2026_07_08.md).
                    Uses the same viewModel.voiceHandle() the header button used to, so voice wiring is unchanged. */}
                <Show when={props.viewModel?.voiceHandle}>
                    <FooterMic viewModel={props.viewModel!} />
                </Show>
            </div>
        </div>
    );
};

AgentFooter.displayName = "AgentFooter";
