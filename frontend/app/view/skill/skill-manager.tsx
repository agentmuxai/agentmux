// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * SkillManager — the Memory → Skills section body. Context-free
 * list/create/edit/delete over the global Skill catalog (skill.catalog.*
 * MuxAPI via SkillCatalogModel). Every row here is global; an agent's own
 * private skills are not shown here. A skill's details also say who gets it
 * without a bind (every agent, new agents).
 */

import { createResource, createSignal, onCleanup, For, Show, type JSX } from "solid-js";
import type { SkillCatalogItem } from "@/types/rpc/SkillCatalogItem";
import { Markdown } from "@/app/element/markdown";
import { PrimitiveListDetail } from "@/app/element/primitive-list-detail";
import { showTextInputContextMenu } from "@/app/store/contextmenu";
import { SkillCatalogModel } from "./skill-model";
import { SkillImportModel } from "./skill-import-model";
import { SkillImportPanel, SkillImportResult } from "./skill-import";
import "./skill-import.scss";
import "../agent/components/AgentPrimitiveModal.scss";
import { Button, Switch } from "@/app/element/ui";
import { focusOnOpen } from "@/util/focusutil";

/** The agents a skill is bound to, each with Unbind. */
function SkillBoundAgents(props: { model: SkillCatalogModel; skill: SkillCatalogItem }): JSX.Element {
    // Asked again when the catalog or the agent list changes: a bind or
    // unbind refreshes the catalog.
    const [bound] = createResource(
        () => ({ skill: props.skill, revision: props.model.revisionAtom(), agents: props.model.agentsAtom() }),
        ({ skill }) => props.model.boundAgents(skill),
    );
    return (
        <Show when={(bound() ?? []).length > 0}>
            <ul class="agent-primitive-modal-bound" aria-label="Agents it is bound to">
                <For each={bound()}>
                    {(agent) => (
                        <li class="agent-primitive-modal-bound-agent">
                            <span>{agent.name}</span>
                            <Button
                                density="compact"
                                aria-label={`Unbind ${agent.name}`}
                                onClick={() => void props.model.unbindFromAgent(props.skill.id, agent.id)}
                            >
                                Unbind
                            </Button>
                        </li>
                    )}
                </For>
            </ul>
        </Show>
    );
}

/** Who gets a skill without a bind: every agent, or the agents created from now on. */
function SkillReach(props: { model: SkillCatalogModel; skill: SkillCatalogItem }): JSX.Element {
    return (
        <div class="agent-primitive-modal-reach" aria-label="Who gets this skill">
            <div class="agent-primitive-modal-reach-row">
                <Switch
                    checked={props.skill.all_agents}
                    ariaLabel="Every agent gets this skill"
                    onChange={(on) => void props.model.setReach(props.skill.id, on, props.skill.new_agents || on)}
                />
                <span>Every agent gets it (the ones to come too)</span>
            </div>
            <div class="agent-primitive-modal-reach-row">
                <Switch
                    checked={props.skill.all_agents || props.skill.new_agents}
                    disabled={props.skill.all_agents}
                    ariaLabel="New agents get this skill"
                    onChange={(on) => void props.model.setReach(props.skill.id, false, on)}
                />
                <span>New agents get it</span>
            </div>
        </div>
    );
}

export const SkillManager = (): JSX.Element => {
    const model = new SkillCatalogModel();
    onCleanup(() => model.dispose());

    // Single-pane — see docs/specs/SPEC_ARMORY_RESPONSIVE_SINGLE_PANE_LAYOUT_2026_07_15.md.
    // The import panel, or an imported skill's "check for changes" (`updateOf` set).
    const [importing, setImporting] = createSignal<{ model: SkillImportModel; updateOf?: SkillCatalogItem } | null>(null);
    const inDetail = () => model.selectedIdAtom() !== null || model.draftAtom() !== null || importing() !== null;
    const imported = (ids: string[]) => {
        setImporting(null);
        void model.refresh();
        if (ids[0]) model.setSelectedId(ids[0]);
    };
    const startImport = () => {
        model.setSelectedId(null);
        model.setDraft(null);
        setImporting({ model: new SkillImportModel(imported) });
    };
    const startCheck = (skill: SkillCatalogItem) => {
        const check = new SkillImportModel(imported);
        setImporting({ model: check, updateOf: skill });
        void check.check(skill.id);
    };
    const handleBack = () => {
        setImporting(null);
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
                            onClick={() => {
                                setImporting(null);
                                model.handleSelect(skill);
                            }}
                        >
                            <span class="agent-primitive-modal-list-item-name">{skill.name}</span>
                            <span class="agent-primitive-modal-list-item-badge">
                                {skill.all_agents
                                    ? "all agents"
                                    : `${skill.bound_count} ${skill.bound_count === 1 ? "agent" : "agents"}`}
                            </span>
                        </button>
                    )}
                </For>
            </Show>

            <button class="agent-primitive-modal-new-btn" onClick={() => model.startNew()}>
                + New skill
            </button>
            <Button class="agent-primitive-modal-new-btn" onClick={startImport}>
                Import a skill…
            </Button>
        </div>
    );

    const detailView = (
        <div class="agent-primitive-modal-detail">
            <Show when={importing()}>
                {(imp) => (
                    <Show
                        when={imp().updateOf}
                        fallback={<SkillImportPanel model={imp().model} onCancel={() => setImporting(null)} />}
                    >
                        {(skill) => (
                            <div class="agent-primitive-modal-form skill-import">
                                <h3 class="agent-primitive-modal-name">{skill().name}: check for changes</h3>
                                <Show when={imp().model.errorAtom()}>
                                    <div class="agent-primitive-modal-error">{imp().model.errorAtom()}</div>
                                </Show>
                                <Show when={imp().model.busyAtom() && !imp().model.previewAtom()}>
                                    <p class="agent-primitive-modal-global-note">Reading the source…</p>
                                </Show>
                                <SkillImportResult model={imp().model} verb="Update" />
                                <div class="agent-primitive-modal-actions">
                                    <Button class="agent-primitive-modal-btn" onClick={() => setImporting(null)}>
                                        Close
                                    </Button>
                                </div>
                            </div>
                        )}
                    </Show>
                )}
            </Show>
            <Show when={!importing()}>
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
                                            {skill().all_agents
                                                ? "Used by every agent"
                                                : `Used by ${skill().bound_count} ${skill().bound_count === 1 ? "agent" : "agents"}`}
                                        </p>
                                        <SkillReach model={model} skill={skill()} />
                                        <SkillBoundAgents model={model} skill={skill()} />
                                        <Show when={skill().source}>
                                            {(src) => (
                                                <>
                                                    <span class="agent-primitive-modal-field-label">Imported from</span>
                                                    <pre class="agent-primitive-modal-field-value">
                                                        {src().location}
                                                        {src().git_ref ? ` (${src().git_ref})` : ""}
                                                        {src().item_key ? `\n${src().item_key}` : ""}
                                                    </pre>
                                                    <div class="agent-primitive-modal-actions">
                                                        <Button class="agent-primitive-modal-btn" onClick={() => startCheck(skill())}>
                                                            Check for changes
                                                        </Button>
                                                    </div>
                                                </>
                                            )}
                                        </Show>
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
                                            <Button
                                                class="agent-primitive-modal-btn"
                                                disabled={!model.bindAgentIdAtom()}
                                                onClick={() => void model.bindToAgent(skill().id, model.bindAgentIdAtom())}
                                            >
                                                Bind
                                            </Button>
                                        </div>
                                        <div class="agent-primitive-modal-actions">
                                            <Button
                                                tone="danger"
                                                class="agent-primitive-modal-btn agent-primitive-modal-btn-danger"
                                                onClick={() => void model.deleteSkill(skill().id)}
                                            >
                                                Delete
                                            </Button>
                                            <Button
                                                class="agent-primitive-modal-btn"
                                                onClick={() => model.startEdit(skill())}
                                            >
                                                Edit
                                            </Button>
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
                                    onContextMenu={showTextInputContextMenu}
                                    placeholder="e.g. pdf-extraction"
                                    required
                                    ref={(el) => focusOnOpen(el, { select: true })}
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
                                        onContextMenu={showTextInputContextMenu}
                                        placeholder="When should this skill be invoked?"
                                    />
                                </Show>
                                <span class="agent-primitive-modal-field-label">Description</span>
                                <input
                                    class="agent-primitive-modal-input"
                                    type="text"
                                    value={draft().description}
                                    onInput={(e) => model.setDraft({ ...draft(), description: e.currentTarget.value })}
                                    onContextMenu={showTextInputContextMenu}
                                />
                                <span class="agent-primitive-modal-field-label">Content</span>
                                <textarea
                                    class="agent-primitive-modal-textarea"
                                    rows={8}
                                    value={draft().content}
                                    onInput={(e) => model.setDraft({ ...draft(), content: e.currentTarget.value })}
                                    onContextMenu={showTextInputContextMenu}
                                    spellcheck={false}
                                />
                                <div class="agent-primitive-modal-actions">
                                    <Button
                                        class="agent-primitive-modal-btn"
                                        onClick={() => model.cancelDraft()}
                                        disabled={model.savingAtom()}
                                    >
                                        Cancel
                                    </Button>
                                    <Button
                                        type="submit"
                                        tone="accent"
                                        class="agent-primitive-modal-btn agent-primitive-modal-btn-primary"
                                        disabled={model.savingAtom() || !draft().name.trim()}
                                    >
                                        {model.savingAtom() ? "Saving…" : "Save"}
                                    </Button>
                                </div>
                            </form>
                        )}
            </Show>
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
