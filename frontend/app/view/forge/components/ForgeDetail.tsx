// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import type { JSX } from "solid-js";
import { For, Show } from "solid-js";
import type { ForgeViewModel, DetailSection } from "../forge-model";
import { PROVIDERS } from "../forge-constants";
import { ForgeContentSection } from "./ForgeContentSection";
import { ForgeSkillsPanel } from "./ForgeSkillsPanel";
import { ForgeHistoryPanel } from "./ForgeHistoryPanel";

const DETAIL_SECTIONS: { id: DetailSection; label: string }[] = [
    { id: "content", label: "Content" },
    { id: "skills", label: "Skills" },
    { id: "history", label: "History" },
];

function DetailSectionButton(props: {
    section: { id: DetailSection; label: string };
    activeSection: DetailSection;
    model: ForgeViewModel;
    agentId: string;
}): JSX.Element {
    const handleClick = async () => {
        props.model.setActiveSection(props.section.id);
        if (props.section.id === "skills") {
            await props.model.loadSkills(props.agentId);
        } else if (props.section.id === "history") {
            await props.model.loadHistory(props.agentId);
        }
    };

    return (
        <button
            class={`forge-section-tab${props.activeSection === props.section.id ? " active" : ""}`}
            onClick={handleClick}
        >
            {props.section.label}
        </button>
    );
}

export function ForgeDetail(props: { model: ForgeViewModel }): JSX.Element {
    const agent = props.model.detailAgentAtom;
    const activeSection = props.model.activeSectionAtom;

    return (
        <Show when={agent()}>
            {(agentVal) => {
                const providerLabel = () => PROVIDERS.find((p) => p.id === agentVal().provider)?.label ?? agentVal().provider;
                const detailTypeBadge = () => {
                    if (agentVal().agent_type === "host") return "HOST";
                    if (agentVal().agent_type === "container") return "CONTAINER";
                    return null;
                };
                return (
                    <div class="forge-pane">
                        <div class="forge-detail-header">
                            <button class="forge-back-btn" onClick={() => props.model.closeDetail()}>
                                &larr; Back
                            </button>
                            <span class="forge-detail-icon">{agentVal().icon}</span>
                            <div class="forge-detail-info">
                                <div class="forge-detail-name-row">
                                    <span class="forge-detail-name">{agentVal().name}</span>
                                    <Show when={detailTypeBadge()}>
                                        <span class={`forge-agent-type-badge forge-agent-type-${agentVal().agent_type}`}>{detailTypeBadge()}</span>
                                    </Show>
                                </div>
                                <span class="forge-detail-sub">
                                    {providerLabel()}
                                    {agentVal().description ? ` \u2022 ${agentVal().description}` : ""}
                                </span>
                            </div>
                            <button class="forge-card-btn" onClick={() => props.model.startEditFromDetail()}>
                                Edit
                            </button>
                        </div>
                        <div class="forge-divider" />
                        <div class="forge-section-tabs">
                            <For each={DETAIL_SECTIONS}>{(s) =>
                                <DetailSectionButton section={s} activeSection={activeSection()} model={props.model} agentId={agentVal().id} />
                            }</For>
                        </div>
                        <div class="forge-section-body">
                            <Show when={activeSection() === "content"}>
                                <ForgeContentSection model={props.model} agentId={agentVal().id} />
                            </Show>
                            <Show when={activeSection() === "skills"}>
                                <ForgeSkillsPanel model={props.model} agentId={agentVal().id} />
                            </Show>
                            <Show when={activeSection() === "history"}>
                                <ForgeHistoryPanel model={props.model} agentId={agentVal().id} />
                            </Show>
                        </div>
                    </div>
                );
            }}
        </Show>
    );
}
