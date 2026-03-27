// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal } from "solid-js";
import type { JSX } from "solid-js";
import { For, Show } from "solid-js";
import type { ForgeViewModel } from "../forge-model";
import { SKILL_TYPES } from "../forge-model";

export function ForgeSkillForm(props: {
    model: ForgeViewModel;
    agentId: string;
    skill: ForgeSkill | null;
    onClose: () => void;
}): JSX.Element {
    const error = props.model.errorAtom;
    const isEdit = () => props.skill != null;

    const [name, setName] = createSignal(props.skill?.name ?? "");
    const [trigger, setTrigger] = createSignal(props.skill?.trigger ?? "");
    const [skillType, setSkillType] = createSignal(props.skill?.skill_type ?? "prompt");
    const [description, setDescription] = createSignal(props.skill?.description ?? "");
    const [content, setContent] = createSignal(props.skill?.content ?? "");

    const handleSubmit = async (e: Event) => {
        e.preventDefault();
        if (!name().trim()) return;
        if (isEdit()) {
            await props.model.updateSkill({
                id: props.skill!.id,
                name: name().trim(),
                trigger: trigger(),
                skill_type: skillType(),
                description: description(),
                content: content(),
            });
        } else {
            await props.model.createSkill({
                agent_id: props.agentId,
                name: name().trim(),
                trigger: trigger(),
                skill_type: skillType(),
                description: description(),
                content: content(),
            });
        }
        props.onClose();
    };

    return (
        <div class="forge-skill-form">
            <div class="forge-skill-form-header">
                <span class="forge-title-sub">{isEdit() ? "Edit Skill" : "New Skill"}</span>
            </div>
            <form onSubmit={handleSubmit}>
                <div class="forge-form-row">
                    <label class="forge-form-label">Name</label>
                    <input
                        class="forge-form-input"
                        value={name()}
                        onInput={(e) => setName(e.currentTarget.value)}
                        placeholder="Skill name"
                        autofocus
                        required
                    />
                </div>
                <div class="forge-form-row">
                    <label class="forge-form-label">Trigger</label>
                    <input
                        class="forge-form-input"
                        value={trigger()}
                        onInput={(e) => setTrigger(e.currentTarget.value)}
                        placeholder="/command-name"
                    />
                </div>
                <div class="forge-form-row">
                    <label class="forge-form-label">Type</label>
                    <select
                        class="forge-form-input forge-form-select"
                        value={skillType()}
                        onInput={(e) => setSkillType(e.currentTarget.value)}
                    >
                        <For each={SKILL_TYPES}>{(t) =>
                            <option value={t}>{t}</option>
                        }</For>
                    </select>
                </div>
                <div class="forge-form-row">
                    <label class="forge-form-label">Description</label>
                    <input
                        class="forge-form-input"
                        value={description()}
                        onInput={(e) => setDescription(e.currentTarget.value)}
                        placeholder="Brief description"
                    />
                </div>
                <div class="forge-form-row forge-form-row-col">
                    <label class="forge-form-label">Content</label>
                    <textarea
                        class="forge-content-textarea forge-skill-content"
                        value={content()}
                        onInput={(e) => setContent(e.currentTarget.value)}
                        placeholder="Skill content (prompt, command, etc.)"
                        spellcheck={false}
                    />
                </div>
                <Show when={error()}>
                    <div class="forge-form-error">{error()}</div>
                </Show>
                <div class="forge-form-actions">
                    <button type="submit" class="forge-btn-primary" disabled={!name().trim()}>
                        {isEdit() ? "Update" : "Create"}
                    </button>
                    <button type="button" class="forge-btn-secondary" onClick={props.onClose}>
                        Cancel
                    </button>
                </div>
            </form>
        </div>
    );
}
