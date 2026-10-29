// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The History pane: the record of what agents did, beside Audit.
// Agents lists every agent's kept history and opens it; Storage shows where it
// is kept, how much, and for how long. Typed shows what the user typed to an agent. See
// docs/specs/SPEC_WARDEN_HISTORY_SECTION_2026_10_18.md.

import { createSignal, type JSX } from "solid-js";

import { SegmentedControl } from "@/app/element/ui";
import { HistoryAgents } from "./history-agents";
import { HistoryStorage } from "./history-storage";
import { HistoryTyped } from "./history-typed";

import "@/app/view/warden-shared/warden-manager-chrome.scss";
import "./history.scss";

type HistoryView = "agents" | "typed" | "storage";

const VIEWS = [
    { value: "agents" as const, label: "Agents", icon: "users" },
    { value: "typed" as const, label: "Typed", icon: "keyboard" },
    { value: "storage" as const, label: "Storage", icon: "hard-drive" },
];

export const HistoryManager = (props: { paneBlockId: string; visible: boolean }): JSX.Element => {
    const [view, setView] = createSignal<HistoryView>("agents");
    return (
        <div class="warden-manager-body warden-history">
            <SegmentedControl options={VIEWS} value={view()} onChange={setView} density="compact" ariaLabel="History view" />
            <div classList={{ "is-hidden": view() !== "agents" }}>
                <HistoryAgents paneBlockId={props.paneBlockId} visible={props.visible && view() === "agents"} />
            </div>
            <div classList={{ "is-hidden": view() !== "typed" }}>
                <HistoryTyped visible={props.visible && view() === "typed"} />
            </div>
            <div classList={{ "is-hidden": view() !== "storage" }}>
                <HistoryStorage visible={props.visible && view() === "storage"} />
            </div>
        </div>
    );
};

HistoryManager.displayName = "HistoryManager";
