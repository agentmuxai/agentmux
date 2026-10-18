// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Warden — History section: the record of what agents did, beside Audit.
// Agents lists every agent's kept history and opens it; Storage shows where it
// is kept, how much, and for how long. See
// docs/specs/SPEC_WARDEN_HISTORY_SECTION_2026_10_18.md.

import { createSignal, type JSX } from "solid-js";

import { SegmentedControl } from "@/app/element/ui";
import { WardenHistoryAgents } from "./warden-history-agents";
import { WardenHistoryStorage } from "./warden-history-storage";

import "@/app/view/warden-shared/warden-manager-chrome.scss";
import "./warden-history.scss";

type HistoryView = "agents" | "storage";

const VIEWS = [
    { value: "agents" as const, label: "Agents", icon: "users" },
    { value: "storage" as const, label: "Storage", icon: "hard-drive" },
];

export const WardenHistoryManager = (props: { wardenBlockId: string; visible: boolean }): JSX.Element => {
    const [view, setView] = createSignal<HistoryView>("agents");
    return (
        <div class="warden-manager-body warden-history">
            <SegmentedControl options={VIEWS} value={view()} onChange={setView} density="compact" ariaLabel="History view" />
            <div classList={{ "is-hidden": view() !== "agents" }}>
                <WardenHistoryAgents wardenBlockId={props.wardenBlockId} visible={props.visible && view() === "agents"} />
            </div>
            <div classList={{ "is-hidden": view() !== "storage" }}>
                <WardenHistoryStorage visible={props.visible && view() === "storage"} />
            </div>
        </div>
    );
};

WardenHistoryManager.displayName = "WardenHistoryManager";
