// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { createSignal } from "solid-js";
import type { JSX } from "solid-js";
import { For, Show } from "solid-js";
import type { ForgeViewModel } from "../forge-model";
import { ForgeSkillCard } from "./ForgeSkillCard";
import { ForgeSkillForm } from "./ForgeSkillForm";

export function ForgeSkillsPanel(props: { model: ForgeViewModel; agentId: string }): JSX.Element {
    const skills = props.model.skillsAtom;
    const loading = props.model.skillsLoadingAtom;
    const editingSkill = props.model.editingSkillAtom;
    const [showForm, setShowForm] = createSignal(false);

    const handleNewSkill = () => {
        props.model.setEditingSkill(null);
        setShowForm(true);
    };

    const handleEditSkill = (skill: ForgeSkill) => {
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
                <ForgeSkillForm
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
                                <ForgeSkillCard
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
