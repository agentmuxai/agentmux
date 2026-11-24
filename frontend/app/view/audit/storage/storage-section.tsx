// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Audit pane's Storage section: where history is kept, how much, and for how long (history-storage.tsx).

import type { JSX } from "solid-js";
import type { AuditSectionProps } from "../audit-section-props";
import { HistoryStorage } from "./history-storage";
import "@/app/view/warden-shared/warden-manager-chrome.scss";
import "../history-sections.scss";

export default function StorageSection(props: AuditSectionProps): JSX.Element {
    return (
        <div class="warden-manager-body warden-history">
            <HistoryStorage visible={props.visible} />
        </div>
    );
}
