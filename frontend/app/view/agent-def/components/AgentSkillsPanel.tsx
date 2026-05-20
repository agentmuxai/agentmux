// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal } from "solid-js";
import type { JSX } from "solid-js";
import { For, Show } from "solid-js";
import type { AgentDefViewModel } from "../agent-def-model";
import { AgentSkillCard } from "./AgentSkillCard";
import { AgentSkillForm } from "./AgentSkillForm";

export function AgentSkillsPanel(props: { model: AgentDefViewModel; agentId: string }): JSX.Element {
    const skills = props.model.skillsAtom;
    const loading = props.model.skillsLoadingAtom;
    const editingSkill = props.model.editingSkillAtom;
    const [showForm, setShowForm] = createSignal(false);

    const handleNewSkill = () => {
        props.model.setEditingSkill(null);
        setShowForm(true);
    };

    const handleEditSkill = (skill: AgentSkill) => {
        props.model.setEditingSkill(skill);
        setShowForm(true);
    };

    const handleCloseForm = () => {
        props.model.setEditingSkill(null);
        setShowForm(false);
    };

    return (
        <Show when={!loading()} fallback={
            <div class="forge-content-loading">Loading skills...</div>
        }>
            <Show when={!showForm()} fallback={
                <AgentSkillForm
                    model={props.model}
                    agentId={props.agentId}
                    skill={editingSkill()}
                    onClose={handleCloseForm}
                />
            }>
                <div class="forge-skills-panel">
                    <Show when={skills().length > 0} fallback={
                        <div class="forge-content-empty">No skills yet</div>
                    }>
                        <div class="forge-skills-list">
                            <For each={skills()}>{(skill) =>
                                <AgentSkillCard
                                    skill={skill}
                                    model={props.model}
                                    onEdit={handleEditSkill}
                                />
                            }</For>
                        </div>
                    </Show>
                    <div class="forge-skills-footer">
                        <button class="forge-btn-primary" onClick={handleNewSkill}>
                            + Add Skill
                        </button>
                    </div>
                </div>
            </Show>
        </Show>
    );
}
