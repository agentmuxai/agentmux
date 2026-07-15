// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentSkillsModal — the "Skills" tab body inside AgentSetupModal.
 * List/create/edit/delete for this agent's own skills, plus a
 * bound-state-aware Bind/Unbind toggle for global ones (driven by
 * `bound_to_agent` — see AgentSkillModel's doc comment). Distinct from the
 * legacy AgentSkillCard/AgentSkillsPanel (the agent-definition
 * `agent_skill_*` surface) — this is the v1 standalone Skill primitive
 * (`skill.*` / `db_skills`).
 */

import { onCleanup, For, Show, type JSX } from "solid-js";
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
    const inDetail = () => model.selectedIdAtom() !== null || model.draftAtom() !== null;
    const handleBack = () => {
        model.setError(null);
        model.setSelectedId(null);
        model.setDraft(null);
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
                            <Show when={skill.is_global}>
                                <span class="agent-primitive-modal-list-item-badge">global</span>
                                <span
                                    class="agent-primitive-modal-list-item-badge"
                                    classList={{ "is-bound": skill.bound_to_agent }}
                                >
                                    {skill.bound_to_agent ? "bound" : "not bound"}
                                </span>
                            </Show>
                        </button>
                    )}
                </For>
            </Show>

            <button class="agent-primitive-modal-new-btn" onClick={() => model.startNew()}>
                + New skill
            </button>
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
            <Show
                when={model.draftAtom()}
                fallback={
                    <Show when={model.selectedAtom()}>
                        {(skill) => (
                                    <div class="agent-primitive-modal-readonly">
                                        <h3 class="agent-primitive-modal-name">{skill().name}</h3>
                                        <Show when={skill().is_global}>
                                            <p class="agent-primitive-modal-global-note">
                                                Global — managed in the Armory. You can bind/unbind it
                                                here, or{" "}
                                                <button
                                                    type="button"
                                                    class="agent-primitive-modal-link-btn"
                                                    onClick={() => void openOrFocusPaneByView("armory")}
                                                >
                                                    edit it there
                                                </button>
                                                .
                                            </p>
                                        </Show>
                                        <Show when={skill().description}>
                                            <span class="agent-primitive-modal-field-label">Description</span>
                                            <pre class="agent-primitive-modal-field-value">{skill().description}</pre>
                                        </Show>
                                        <Show when={skill().trigger}>
                                            <span class="agent-primitive-modal-field-label">Trigger</span>
                                            <pre class="agent-primitive-modal-field-value">{skill().trigger}</pre>
                                        </Show>
                                        <span class="agent-primitive-modal-field-label">Content</span>
                                        <pre class="agent-primitive-modal-field-value">{skill().content || "(none)"}</pre>
                                        <div class="agent-primitive-modal-actions">
                                            <Show
                                                when={!skill().is_global}
                                                fallback={
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
                                                }
                                            >
                                                <button
                                                    class="agent-primitive-modal-btn agent-primitive-modal-btn-danger"
                                                    onClick={() => void model.deleteSkill(skill().id)}
                                                >
                                                    Delete
                                                </button>
                                                <button
                                                    class="agent-primitive-modal-btn"
                                                    onClick={() => model.startEdit(skill())}
                                                >
                                                    Edit
                                                </button>
                                            </Show>
                                        </div>
                                    </div>
                                )}
                            </Show>
                        }
                    >
                        {(draft) => (
                            <form
                                class="agent-primitive-modal-form"
                                onSubmit={(e) => { e.preventDefault(); void model.saveDraft(); }}
                            >
                                <span class="agent-primitive-modal-field-label">Name *</span>
                                <input
                                    class="agent-primitive-modal-input"
                                    type="text"
                                    value={draft().name}
                                    onInput={(e) => model.setDraft({ ...draft(), name: e.currentTarget.value })}
                                    placeholder="e.g. pdf-extraction"
                                    required
                                />
                                <span class="agent-primitive-modal-field-label">Trigger</span>
                                <input
                                    class="agent-primitive-modal-input"
                                    type="text"
                                    value={draft().trigger}
                                    onInput={(e) => model.setDraft({ ...draft(), trigger: e.currentTarget.value })}
                                    placeholder="When should this skill be invoked?"
                                />
                                <span class="agent-primitive-modal-field-label">Description</span>
                                <input
                                    class="agent-primitive-modal-input"
                                    type="text"
                                    value={draft().description}
                                    onInput={(e) => model.setDraft({ ...draft(), description: e.currentTarget.value })}
                                />
                                <span class="agent-primitive-modal-field-label">Content</span>
                                <textarea
                                    class="agent-primitive-modal-textarea"
                                    rows={8}
                                    value={draft().content}
                                    onInput={(e) => model.setDraft({ ...draft(), content: e.currentTarget.value })}
                                    spellcheck={false}
                                />
                                <div class="agent-primitive-modal-actions">
                                    <button
                                        type="button"
                                        class="agent-primitive-modal-btn"
                                        onClick={() => model.cancelDraft()}
                                        disabled={model.savingAtom()}
                                    >
                                        Cancel
                                    </button>
                                    <button
                                        type="submit"
                                        class="agent-primitive-modal-btn agent-primitive-modal-btn-primary"
                                        disabled={model.savingAtom() || !draft().name.trim()}
                                    >
                                        {model.savingAtom() ? "Saving…" : "Save"}
                                    </button>
                                </div>
                            </form>
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
