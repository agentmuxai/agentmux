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
import { Markdown } from "@/app/element/markdown";
import { PrimitiveListDetail } from "@/app/element/primitive-list-detail";
import { SkillCatalogModel } from "./skill-model";
import "../agent/components/AgentPrimitiveModal.scss";

export const SkillManager = (): JSX.Element => {
    const model = new SkillCatalogModel();
    onCleanup(() => model.dispose());

    // Single-pane — see docs/specs/SPEC_ARMORY_RESPONSIVE_SINGLE_PANE_LAYOUT_2026_07_15.md.
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
                            <span class="agent-primitive-modal-list-item-badge">
                                {skill.bound_count} {skill.bound_count === 1 ? "agent" : "agents"}
                            </span>
                        </button>
                    )}
                </For>
            </Show>

            <button class="agent-primitive-modal-new-btn" onClick={() => model.startNew()}>
                + New skill
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
                                        <p class="agent-primitive-modal-global-note">
                                            Used by {skill().bound_count} {skill().bound_count === 1 ? "agent" : "agents"}
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
                                        <span class="agent-primitive-modal-field-label">Bind to agent</span>
                                        <div class="agent-primitive-modal-bind-row">
                                            <select
                                                class="agent-primitive-modal-input"
                                                value={model.bindAgentIdAtom()}
                                                onChange={(e) => model.setBindAgentId(e.currentTarget.value)}
                                            >
                                                <option value="">Select an agent…</option>
                                                <For each={model.agentsAtom()}>
                                                    {(agent) => <option value={agent.id}>{agent.name}</option>}
                                                </For>
                                            </select>
                                            <button
                                                class="agent-primitive-modal-btn"
                                                disabled={!model.bindAgentIdAtom()}
                                                onClick={() => void model.bindToAgent(skill().id, model.bindAgentIdAtom())}
                                            >
                                                Bind
                                            </button>
                                        </div>
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
                                <span class="agent-primitive-modal-field-label">Format</span>
                                <select
                                    class="agent-primitive-modal-input"
                                    value={draft().skill_type || "prompt"}
                                    onChange={(e) => model.setDraft({ ...draft(), skill_type: e.currentTarget.value })}
                                >
                                    <option value="prompt">Slash command (/trigger)</option>
                                    <option value="agent-skill">Agent Skill (SKILL.md) — beta ABF format</option>
                                </select>
                                <Show when={(draft().skill_type || "prompt") !== "agent-skill"}>
                                    <span class="agent-primitive-modal-field-label">Trigger</span>
                                    <input
                                        class="agent-primitive-modal-input"
                                        type="text"
                                        value={draft().trigger}
                                        onInput={(e) => model.setDraft({ ...draft(), trigger: e.currentTarget.value })}
                                        placeholder="When should this skill be invoked?"
                                    />
                                </Show>
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

SkillManager.displayName = "SkillManager";
