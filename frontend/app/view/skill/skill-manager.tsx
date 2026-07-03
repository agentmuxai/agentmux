// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * SkillManager — the Armory's "Skills" tab body. Context-free
 * list/create/edit/delete over the global Skill catalog (skill.catalog.*
 * App API via SkillCatalogModel). Every row here is global — per-agent
 * private skills live in the Agent-setup modal (AgentSkillsModal), not
 * here.
 */

import { onCleanup, For, Show, type JSX } from "solid-js";
import { SkillCatalogModel } from "./skill-model";
import "../agent/components/AgentPrimitiveModal.scss";

export const SkillManager = (): JSX.Element => {
    const model = new SkillCatalogModel();
    onCleanup(() => model.dispose());

    return (
        <div class="agent-primitive-modal">
            <Show when={model.errorAtom()}>
                <div class="agent-primitive-modal-error">{model.errorAtom()}</div>
            </Show>

            <div class="agent-primitive-modal-body">
                <div class="agent-primitive-modal-list">
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
                                </button>
                            )}
                        </For>
                    </Show>

                    <button class="agent-primitive-modal-new-btn" onClick={() => model.startNew()}>
                        + New skill
                    </button>
                </div>

                <div class="agent-primitive-modal-detail">
                    <Show
                        when={model.draftAtom()}
                        fallback={
                            <Show
                                when={model.selectedAtom()}
                                fallback={
                                    <div class="agent-primitive-modal-empty">
                                        Select a skill from the list, or create a new one. Skills
                                        created here are global — visible to every agent.
                                    </div>
                                }
                            >
                                {(skill) => (
                                    <div class="agent-primitive-modal-readonly">
                                        <h3 class="agent-primitive-modal-name">{skill().name}</h3>
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
            </div>
        </div>
    );
};

SkillManager.displayName = "SkillManager";
