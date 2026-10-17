// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Button } from "@/app/element/ui";
import { incidentMessage, incidentReport, recentIncidents, type Incident } from "@/app/init/incidents";
import { writeText } from "@/util/clipboard";
import { createSignal, For, Show, type JSX } from "solid-js";

/**
 * What AgentMux recovered from, kept after its notice is closed: the backend
 * status panel's "Recent incidents", newest first, each with when, what, why,
 * what was saved, and a report to copy
 * (docs/specs/SPEC_RECOVERY_NOTICES_AND_TOASTS_2026_10_16.md §4.3).
 */
export const RecentIncidents = (): JSX.Element => (
    <Show when={recentIncidents().length > 0}>
        <div class="status-bar-popover-section-title">Recent incidents</div>
        <div class="status-bar-incidents" role="list" aria-label="Recent incidents">
            <For each={recentIncidents()}>{(i) => <IncidentRow incident={i} />}</For>
        </div>
        <div class="status-bar-popover-divider" />
    </Show>
);

RecentIncidents.displayName = "RecentIncidents";

const IncidentRow = (props: { incident: Incident }): JSX.Element => {
    const [copied, setCopied] = createSignal<"" | "Copied" | "Couldn't copy">("");
    const open = () => !props.incident.acknowledged_at_ms;
    return (
        <div class={`status-bar-incident${open() ? " status-bar-incident--open" : ""}`} role="listitem">
            <div class="status-bar-incident-head">
                <span class="status-bar-incident-what">{props.incident.what}</span>
                <span class="status-bar-incident-when">{new Date(props.incident.at_ms).toLocaleString()}</span>
            </div>
            <div class="status-bar-incident-text">{incidentMessage(props.incident)}</div>
            <Button
                density="compact"
                tone="quiet"
                class="status-bar-incident-copy"
                onClick={() => {
                    // Through the host: CEF blocks navigator.clipboard.
                    writeText(incidentReport(props.incident)).then(
                        () => setCopied("Copied"),
                        () => setCopied("Couldn't copy")
                    );
                }}
            >
                {copied() || "Copy report"}
            </Button>
        </div>
    );
};
