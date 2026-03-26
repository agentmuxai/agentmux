// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { createSignal } from "solid-js";
import type { JSX } from "solid-js";
import { Show } from "solid-js";
import type { ForgeViewModel } from "../forge-model";

export function ForgeSkillCard(props: {
    skill: ForgeSkill;
    model: ForgeViewModel;
    onEdit: (skill: ForgeSkill) => void;
}): JSX.Element {
    const [confirming, setConfirming] = createSignal(false);

    const handleDelete = async () => {
        if (!confirming()) {
            setConfirming(true);
            return;
        }
        setConfirming(false);
        await props.model.deleteSkill(props.skill.id);
    };

    return (
        <div class="forge-skill-card">
            <div class="forge-skill-card-info">
                <div class="forge-skill-card-top">
                    <span class="forge-skill-card-name">{props.skill.name}</span>
                    <span class="forge-skill-type-badge">{props.skill.skill_type}</span>
                </div>
                <Show when={props.skill.trigger}>
                    <span class="forge-skill-card-trigger">/{props.skill.trigger}</span>
                </Show>
                <Show when={props.skill.description}>
                    <span class="forge-skill-card-desc">{props.skill.description}</span>
                </Show>
            </div>
            <div class="forge-card-actions">
                <button class="forge-card-btn" onClick={() => props.onEdit(props.skill)} title="Edit">
                    Edit
                </button>
                <button
                    class={`forge-card-btn forge-card-btn-delete${confirming() ? " confirming" : ""}`}
                    onClick={handleDelete}
                    onBlur={() => setConfirming(false)}
                    title={confirming() ? "Click again to confirm" : "Delete"}
                >
                    {confirming() ? "Sure?" : "\u2715"}
                </button>
            </div>
        </div>
    );
}
