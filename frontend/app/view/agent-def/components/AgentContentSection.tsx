// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { JSX } from "solid-js";
import { For, Show } from "solid-js";
import type { AgentDefViewModel, ContentTabId } from "../agent-def-model";
import { CONTENT_TABS, CONTENT_TAB_LABELS } from "../agent-def-model";
import { ContentEditor } from "./ContentEditor";

function ContentTabButton(props: {
    tab: ContentTabId;
    activeTab: ContentTabId;
    model: AgentDefViewModel;
}): JSX.Element {
    const handleClick = async () => {
        props.model.setActiveTab(props.tab);
    };

    return (
        <button
            class={`forge-content-tab${props.activeTab === props.tab ? " active" : ""}`}
            onClick={handleClick}
        >
            {CONTENT_TAB_LABELS[props.tab]}
        </button>
    );
}

export function AgentContentSection(props: { model: AgentDefViewModel; agentId: string }): JSX.Element {
    const contentMap = props.model.contentAtom;
    const activeTab = props.model.activeTabAtom;
    const contentLoading = props.model.contentLoadingAtom;

    return (
        <>
            <div class="forge-content-tabs">
                <For each={CONTENT_TABS}>{(tab) =>
                    <ContentTabButton tab={tab} activeTab={activeTab()} model={props.model} />
                }</For>
            </div>
            <div class="forge-content-body">
                <Show when={!contentLoading()} fallback={
                    <div class="forge-content-loading">Loading...</div>
                }>
                    <ContentEditor
                        agentId={props.agentId}
                        contentType={activeTab()}
                        content={contentMap()[activeTab()]}
                        model={props.model}
                    />
                </Show>
            </div>
        </>
    );
}
