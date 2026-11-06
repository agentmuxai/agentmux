// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What the agent picker holds (AgentPicker.tsx lays it out): the filter and the sort, the definitions, and the
// modules that feed it (readiness, install checks, system tools, templates, launch), built in this order, and the
// pane's zoom.

import { readZoom } from "@/app/store/zoom-factor";
import { createMemo, createSignal } from "solid-js";
import type { AgentViewModel } from "../agent-model";
import { type AgentSortOption } from "./AgentPickerFilterBar";
import { useAgentDefinitions, useInstanceOpenDefinitions } from "./agent-definition-hooks";
import { useInstallStates } from "./agent-picker-install";
import { createPickerLaunch } from "./agent-picker-launch";
import { createPrereqs } from "./agent-picker-prereqs";
import { usePickerReadiness } from "./agent-picker-readiness";
import { loadStoredSort, storeSort } from "./agent-picker-sort";
import { createTemplates } from "./agent-picker-templates";

export interface AgentPickerProps {
    model: AgentViewModel;
}

export function createAgentPicker(props: AgentPickerProps) {
    // Filter-bar query (SPEC_AGENT_PICKER_FILTER_SEARCH_2026_08_17.md) —
    // narrows MyAgentsList only, not the template grid below it (Q1,
    // confirmed by the human operator): the ask was specifically to find
    // an *existing agent*, and a shared query risks the template grid
    // silently shrinking for an unrelated reason right next to it.
    const [filterQuery, setFilterQuery] = createSignal("");
    // Sort control, far right of the filter bar (SPEC follow-up — see
    // docs/reports/REPORT_AGENT_PICKER_FIELD_ORDER_SORT_AND_DATA_GAPS_AUDIT_2026_08_24.md §3).
    // Same narrowing-scope decision as filterQuery above: only MyAgentsList,
    // not the template grid.
    const [sortBy, setSortByState] = createSignal<AgentSortOption>(loadStoredSort());
    const setSortBy = (sort: AgentSortOption) => {
        setSortByState(sort);
        storeSort(sort);
    };
    const [agents, definitionsLoading] = useAgentDefinitions();

    // The loading cover, the install checks, the system-tool probe, what a click launches and the New agent half
    // (each is a module of its own: agent-picker-*.ts).
    const readiness = usePickerReadiness(props.model.blockId, agents, definitionsLoading);
    const { pickerReadiness } = readiness;

    // Reactive map of definition_id → blockId for panes currently open.
    const [openDefinitions, refreshOpenDefinitions, openLocations, resolveOpenDefinitions] =
        useInstanceOpenDefinitions();
    const install = useInstallStates(agents);
    const prereqs = createPrereqs();
    const templates = createTemplates({
        agents,
        install,
        clearNodejsError: () => launch.setNodejsError(null),
    });
    const launch = createPickerLaunch(props, { agents, refreshOpenDefinitions, forgetTemplate: templates.forgetTemplate });

    // Per-pane zoom — read `term:zoom` from block meta and apply CSS
    // `zoom` on the root, matching AgentPresentationView (chat mode).
    // The universal framework (Ctrl+/-/0 in keymodel, Ctrl+Wheel in
    // app.tsx) writes `term:zoom` for any block whose `viewType ===
    // "agent"`; the picker just needs to read + apply.
    const block = props.model.blockAtom;
    const zoomFactor = createMemo(() => readZoom(block()?.meta));
    return {
        filterQuery, setFilterQuery, sortBy, setSortBy, agents, definitionsLoading, readiness, pickerReadiness,
        openDefinitions, openLocations, resolveOpenDefinitions, install, prereqs, templates, launch, zoomFactor,
    };
}

export type AgentPickerState = ReturnType<typeof createAgentPicker>;
