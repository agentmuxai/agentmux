// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Audit pane's Conversations section: every agent's kept conversations, to open and read (history-agents.tsx).

import type { JSX } from "solid-js";
import type { AuditSectionProps } from "../audit-section-props";
import { HistoryAgents } from "./history-agents";
import "@/app/view/warden-shared/warden-manager-chrome.scss";
import "../history-sections.scss";

export default function ConversationsSection(props: AuditSectionProps): JSX.Element {
    return (
        <div class="warden-manager-body warden-history">
            <HistoryAgents paneBlockId={props.paneBlockId} visible={props.visible} />
        </div>
    );
}
