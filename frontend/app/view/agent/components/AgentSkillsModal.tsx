// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentSkillsModal — the "Skills" tab body inside AgentStashModal. A
 * reactive, read-only list of every skill visible to this agent (global +
 * this agent's own, if any exist), plus a bound-state-aware Bind/Unbind
 * toggle for global ones (driven by `bound_to_agent` — see AgentSkillModel's
 * doc comment). Skills are authored in the Armory, not here. Distinct from
 * the legacy AgentSkillCard/AgentSkillsPanel (the agent-definition
 * `agent_skill_*` surface) — this is the v1 standalone Skill primitive
 * (`skill.*` / `db_skills`).
 */

import { onCleanup, For, Show, type JSX } from "solid-js";
import { Markdown } from "@/app/element/markdown";
import { PrimitiveListDetail } from "@/app/element/primitive-list-detail";
import { openOrFocusPaneByView } from "@/app/store/global";
import { AgentSkillModel } from "../agent-skill-model";
import "./AgentPrimitiveModal.scss";

interface AgentSkillsModalProps {
    agentId: string;
}

export const AgentSkillsModal = (props: AgentSkillsModalProps): JSX.Element => {
    const model = new AgentSkillModel(props.agentId);
    onCleanup(() => model.dispose());

    // Single-pane — see docs/specs/SPEC_ARMORY_RESPONSIVE_SINGLE_PANE_LAYOUT_2026_07_15.md
    // §5 (shared with Armory's Skills/MCP Servers tabs; applied here too since
    // there's no evidence the split was intentional in this modal specifically).
    const inDetail = () => model.selectedIdAtom() !== null;
    const handleBack = () => {
        model.setError(null);
        model.setSelectedId(null);
    };

    const listView = (
        <div class="agent-primitive-modal-list">
            <Show when={model.errorAtom()}>
                <div class="agent-primitive-modal-error">{model.errorAtom()}</div>
            </Show>
            <Show
                when={model.skillsAtom().length > 0}
                fallback={<div class="agent-primitive-modal-list-empty">No skills yet</div>}
            >
                <For each={model.skillsAtom()}>
                    {(skill) => (
                        <button
                            class="agent-primitive-modal-list-item"
                            classList={{ "is-selected": model.selectedIdAtom() === skill.id }}
                            onClick={() => model.handleSelect(skill)}
                        >
                            <span class="agent-primitive-modal-list-item-name">{skill.name}</span>
                            <span
                                class="agent-primitive-modal-list-item-badge"
                                classList={{ "is-bound": skill.bound_to_agent }}
                            >
                                {skill.bound_to_agent ? "bound" : "not bound"}
                            </span>
                        </button>
                    )}
                </For>
            </Show>

            <button
                class="agent-primitive-modal-new-btn"
                onClick={() => void openOrFocusPaneByView("armory")}
            >
                Browse the Armory catalog →
            </button>
        </div>
    );

    const detailView = (
        <div class="agent-primitive-modal-detail">
            <Show when={model.errorAtom()}>
                <div class="agent-primitive-modal-error">{model.errorAtom()}</div>
            </Show>
            <Show when={model.selectedAtom()}>
                {(skill) => (
                    <div class="agent-primitive-modal-readonly">
                        <h3 class="agent-primitive-modal-name">{skill().name}</h3>
                        <p class="agent-primitive-modal-global-note">
                            Global — managed in the Armory. You can bind/unbind it here, or{" "}
                            <button
                                type="button"
                                class="agent-primitive-modal-link-btn"
                                onClick={() => void openOrFocusPaneByView("armory")}
                            >
                                edit it there
                            </button>
                            .
                        </p>
                        <Show when={skill().description}>
                            <span class="agent-primitive-modal-field-label">Description</span>
                            <pre class="agent-primitive-modal-field-value">{skill().description}</pre>
                        </Show>
                        <span class="agent-primitive-modal-field-label">Format</span>
                        <pre class="agent-primitive-modal-field-value">
                            {skill().skill_type === "agent-skill"
                                ? "Agent Skill (SKILL.md)"
                                : "Slash command"}
                        </pre>
                        <Show when={skill().skill_type !== "agent-skill" && skill().trigger}>
                            <span class="agent-primitive-modal-field-label">Trigger</span>
                            <pre class="agent-primitive-modal-field-value">{skill().trigger}</pre>
                        </Show>
                        <span class="agent-primitive-modal-field-label">Content</span>
                        <Show
                            when={skill().content}
                            fallback={<pre class="agent-primitive-modal-field-value">(none)</pre>}
                        >
                            <div class="agent-primitive-modal-field-value agent-primitive-modal-field-value--markdown">
                                <Markdown text={skill().content} scrollable={false} />
                            </div>
                        </Show>
                        <p class="agent-primitive-modal-caveat">
                            Note: every global skill is currently applied to every agent
                            regardless of bind state — unbinding here updates what shows as
                            "in use" but does not yet remove it from this agent's live config.
                        </p>
                        <div class="agent-primitive-modal-actions">
                            <Show
                                when={skill().bound_to_agent}
                                fallback={
                                    <button
                                        class="agent-primitive-modal-btn agent-primitive-modal-btn-primary"
                                        onClick={() => void model.bind(skill().id)}
                                    >
                                        Bind
                                    </button>
                                }
                            >
                                <button
                                    class="agent-primitive-modal-btn"
                                    onClick={() => void model.unbind(skill().id)}
                                >
                                    Unbind
                                </button>
                            </Show>
                        </div>
                    </div>
                )}
            </Show>
        </div>
    );

    return (
        <PrimitiveListDetail
            showDetail={inDetail()}
            backLabel="Skills"
            onBack={handleBack}
            list={listView}
            detail={detailView}
        />
    );
};

AgentSkillsModal.displayName = "AgentSkillsModal";
