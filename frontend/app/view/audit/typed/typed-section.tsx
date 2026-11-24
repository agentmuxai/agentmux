// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Audit pane's Typed section: what the person typed to an agent (history-typed.tsx).

import type { JSX } from "solid-js";
import type { AuditSectionProps } from "../audit-section-props";
import { HistoryTyped } from "./history-typed";
import "@/app/view/warden-shared/warden-manager-chrome.scss";
import "../history-sections.scss";

export default function TypedSection(props: AuditSectionProps): JSX.Element {
    return (
        <div class="warden-manager-body warden-history">
            <HistoryTyped visible={props.visible} />
        </div>
    );
}
