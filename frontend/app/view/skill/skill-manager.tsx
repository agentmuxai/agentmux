// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * SkillManager — the Memory → Skills section body. Context-free
 * list/create/edit/delete over the global Skill catalog (skill.catalog.*
 * MuxAPI via SkillCatalogModel). Every row here is global; an agent's own
 * private skills are not shown here. A skill's details also say who gets it
 * without a bind (every agent, new agents).
 */

import { createResource, createSignal, onCleanup, For, Show, type Accessor, type JSX, type Setter } from "solid-js";
import type { SkillCatalogItem } from "@/types/rpc/SkillCatalogItem";
import { Markdown } from "@/app/element/markdown";
import { PrimitiveListDetail } from "@/app/element/primitive-list-detail";
import { showTextInputContextMenu } from "@/app/store/contextmenu";
import { SkillCatalogModel, type SkillDraft } from "./skill-model";
import { SkillImportModel } from "./skill-import-model";
import { SkillImportPanel, SkillImportResult } from "./skill-import";
import { SkillMatrix } from "./skill-matrix";
import { AppFilterRow, SkillAppDetail } from "./skill-app";
import { appPlatform, appStatus, filterSkills, firstRoute, type AppFilter } from "./skill-app-model";
import "./skill-import.scss";
import "../agent/components/AgentPrimitiveModal.scss";
import { Button, Card, CardGrid, Pill, Switch, TextArea, TextInput } from "@/app/element/ui";
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

type Importing = { model: SkillImportModel; updateOf?: SkillCatalogItem };

/** The catalog model and which side panel (import, check for changes, matrix) is open over it. */
interface ManagerState {
    model: SkillCatalogModel;
    importing: Accessor<Importing | null>;
    setImporting: Setter<Importing | null>;
    showMatrix: Accessor<boolean>;
    setShowMatrix: Setter<boolean>;
    filter: Accessor<AppFilter>;
    setFilter: Setter<AppFilter>;
}

function createManagerState(): ManagerState {
    const model = new SkillCatalogModel();
    onCleanup(() => model.dispose());

    // Single-pane — see docs/specs/SPEC_ARMORY_RESPONSIVE_SINGLE_PANE_LAYOUT_2026_07_15.md.
    // The import panel, or an imported skill's "check for changes" (`updateOf` set).
    const [importing, setImporting] = createSignal<Importing | null>(null);
    const [showMatrix, setShowMatrix] = createSignal(false);
    const [filter, setFilter] = createSignal<AppFilter>("all");
    return { model, importing, setImporting, showMatrix, setShowMatrix, filter, setFilter };
}

const inDetail = (m: ManagerState) =>
    m.model.selectedIdAtom() !== null || m.model.draftAtom() !== null || m.importing() !== null || m.showMatrix();

function importedSkills(m: ManagerState, ids: string[]): void {
    m.setImporting(null);
    void m.model.refresh();
    if (ids[0]) m.model.setSelectedId(ids[0]);
}

function startImport(m: ManagerState): void {
    m.setShowMatrix(false);
    m.model.setSelectedId(null);
    m.model.setDraft(null);
    m.setImporting({ model: new SkillImportModel((ids) => importedSkills(m, ids)) });
}

function startCheck(m: ManagerState, skill: SkillCatalogItem): void {
    const check = new SkillImportModel((ids) => importedSkills(m, ids));
    m.setImporting({ model: check, updateOf: skill });
    void check.check(skill.id);
}

function goBack(m: ManagerState): void {
    m.setShowMatrix(false);
    m.setImporting(null);
    m.model.setError(null);
    m.model.setSelectedId(null);
    m.model.setDraft(null);
}

/** Who gets a skill, as the short facts a card shows: every agent, the new ones, or how many it is bound to. */
function reachFacts(skill: SkillCatalogItem): string[] {
    if (skill.all_agents) return ["Every agent"];
    const bound = `${skill.bound_count} ${skill.bound_count === 1 ? "agent" : "agents"}`;
    if (!skill.new_agents) return [bound];
    return skill.bound_count > 0 ? ["New agents", bound] : ["New agents"];
}

function SkillCard(p: { m: ManagerState; skill: SkillCatalogItem }): JSX.Element {
    const app = () => p.skill.app;
    const status = () => (app() ? appStatus(app()!, appPlatform()) : null);
    // An App skill's card leads with how well it is tested and its route; where it came from is in its detail.
    const facts = () => [
        ...(app() ? [<span>{status()!.label}</span>] : []),
        ...(app() && firstRoute(app()!) ? [<span>route: {firstRoute(app()!)}</span>] : []),
        ...reachFacts(p.skill).map((fact) => <span>{fact}</span>),
        ...(p.skill.source && !app() ? [<span>Imported from {p.skill.source.location}</span>] : []),
    ];
    return (
        <Card
            class="skill-card"
            icon={<i class="fa-solid fa-wand-magic-sparkles" />}
            title={p.skill.name}
            subtitle={
                app() ? (
                    <>
                        <Pill tone="accent">App skill</Pill> {app()!.vendor}
                    </>
                ) : (
                    p.skill.description || undefined
                )
            }
            status={status() ? { tone: status()!.tone, label: status()!.label } : undefined}
            attention={status()?.tone === "error"}
            meta={facts()}
            onSelect={() => {
                p.m.setImporting(null);
                p.m.setShowMatrix(false);
                p.m.model.handleSelect(p.skill);
            }}
        />
    );
}

function SkillList(p: { m: ManagerState }): JSX.Element {
    const model = p.m.model;
    const shown = () => filterSkills(model.skillsAtom(), p.m.filter());
    return (
        <div class="agent-primitive-modal-list ui-density-compact">
            <div class="primitive-card-actions">
                <Button density="compact" onClick={() => model.startNew()}>
                    + New skill
                </Button>
                <Button density="compact" onClick={() => startImport(p.m)}>
                    Import a skill…
                </Button>
                <Button
                    density="compact"
                    onClick={() => {
                        model.setSelectedId(null);
                        model.setDraft(null);
                        p.m.setImporting(null);
                        p.m.setShowMatrix(true);
                    }}
                >
                    Agents and skills
                </Button>
            </div>
            <Show when={model.errorAtom()}>
                <div class="agent-primitive-modal-error">{model.errorAtom()}</div>
            </Show>
            <div class="skill-app-filter">
                <AppFilterRow value={p.m.filter()} onChange={p.m.setFilter} />
            </div>
            <Show
                when={shown().length > 0}
                fallback={
                    <div class="agent-primitive-modal-list-empty">
                        {p.m.filter() === "app" ? "No App skills yet. Import one, or write your own." : "No skills yet"}
                    </div>
                }
            >
                <div class="primitive-card-body">
                    <CardGrid
                        ariaLabel="Skills"
                        items={shown()}
                        keyOf={(skill) => skill.id}
                        renderCard={(skill) => <SkillCard m={p.m} skill={skill} />}
                    />
                </div>
            </Show>
        </div>
    );
}

/** An imported skill's "check for changes": what its source says now, with Update. */
function SkillCheck(p: { m: ManagerState; imp: Importing; skill: SkillCatalogItem }): JSX.Element {
    return (
        <div class="agent-primitive-modal-form skill-import">
            <h3 class="agent-primitive-modal-name">{p.skill.name}: check for changes</h3>
            <Show when={p.imp.model.errorAtom()}>
                <div class="agent-primitive-modal-error">{p.imp.model.errorAtom()}</div>
            </Show>
            <Show when={p.imp.model.busyAtom() && !p.imp.model.previewAtom()}>
                <p class="agent-primitive-modal-global-note">Reading the source…</p>
            </Show>
            <SkillImportResult model={p.imp.model} verb="Update" />
            <div class="agent-primitive-modal-actions">
                <Button class="agent-primitive-modal-btn" onClick={() => p.m.setImporting(null)}>
                    Close
                </Button>
            </div>
        </div>
    );
}

function SkillReadOnly(p: { m: ManagerState; skill: Accessor<SkillCatalogItem> }): JSX.Element {
    const model = p.m.model;
    const skill = p.skill;
    return (
        <div class="agent-primitive-modal-readonly">
            <h3 class="agent-primitive-modal-name">{skill().name}</h3>
            <p class="agent-primitive-modal-global-note">
                {skill().all_agents
                    ? "Used by every agent"
                    : `Used by ${skill().bound_count} ${skill().bound_count === 1 ? "agent" : "agents"}`}
            </p>
            <Show when={skill().app}>{(app) => <SkillAppDetail skillId={skill().id} app={app()} />}</Show>
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
                            <Button class="agent-primitive-modal-btn" onClick={() => startCheck(p.m, skill())}>
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
                {skill().skill_type === "agent-skill" ? "Agent Skill (SKILL.md)" : "Slash command"}
            </pre>
            <Show when={skill().skill_type !== "agent-skill" && skill().trigger}>
                <span class="agent-primitive-modal-field-label">Trigger</span>
                <pre class="agent-primitive-modal-field-value">{skill().trigger}</pre>
            </Show>
            <span class="agent-primitive-modal-field-label">Content</span>
            <Show when={skill().content} fallback={<pre class="agent-primitive-modal-field-value">(none)</pre>}>
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
                    <For each={model.agentsAtom()}>{(agent) => <option value={agent.id}>{agent.name}</option>}</For>
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
                <Button class="agent-primitive-modal-btn" onClick={() => model.startEdit(skill())}>
                    Edit
                </Button>
            </div>
        </div>
    );
}

function SkillForm(p: { model: SkillCatalogModel; draft: Accessor<SkillDraft> }): JSX.Element {
    const model = p.model;
    const draft = p.draft;
    return (
        <form
            class="agent-primitive-modal-form"
            onSubmit={(e) => {
                e.preventDefault();
                void model.saveDraft();
            }}
        >
            <span class="agent-primitive-modal-field-label">Name *</span>
            <TextInput
                variant="plain"
                class="agent-primitive-modal-input"
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
                <TextInput
                    variant="plain"
                    class="agent-primitive-modal-input"
                    value={draft().trigger}
                    onInput={(e) => model.setDraft({ ...draft(), trigger: e.currentTarget.value })}
                    onContextMenu={showTextInputContextMenu}
                    placeholder="When should this skill be invoked?"
                />
            </Show>
            <span class="agent-primitive-modal-field-label">Description</span>
            <TextInput
                variant="plain"
                class="agent-primitive-modal-input"
                value={draft().description}
                onInput={(e) => model.setDraft({ ...draft(), description: e.currentTarget.value })}
                onContextMenu={showTextInputContextMenu}
            />
            <span class="agent-primitive-modal-field-label">Content</span>
            <TextArea
                variant="plain"
                class="agent-primitive-modal-textarea"
                rows={8}
                value={draft().content}
                onInput={(e) => model.setDraft({ ...draft(), content: e.currentTarget.value })}
                onContextMenu={showTextInputContextMenu}
                spellcheck={false}
            />
            <div class="agent-primitive-modal-actions">
                <Button class="agent-primitive-modal-btn" onClick={() => model.cancelDraft()} disabled={model.savingAtom()}>
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
    );
}

function SkillDetail(p: { m: ManagerState }): JSX.Element {
    const model = p.m.model;
    return (
        <div class="agent-primitive-modal-detail">
            <Show when={p.m.importing()}>
                {(imp) => (
                    <Show
                        when={imp().updateOf}
                        fallback={<SkillImportPanel model={imp().model} onCancel={() => p.m.setImporting(null)} />}
                    >
                        {(skill) => <SkillCheck m={p.m} imp={imp()} skill={skill()} />}
                    </Show>
                )}
            </Show>
            <Show when={p.m.showMatrix() && !p.m.importing()}>
                <SkillMatrix catalog={model} />
            </Show>
            <Show when={!p.m.importing() && !p.m.showMatrix()}>
                <Show when={model.errorAtom()}>
                    <div class="agent-primitive-modal-error">{model.errorAtom()}</div>
                </Show>
                <Show
                    when={model.draftAtom()}
                    fallback={<Show when={model.selectedAtom()}>{(skill) => <SkillReadOnly m={p.m} skill={skill} />}</Show>}
                >
                    {(draft) => <SkillForm model={model} draft={draft} />}
                </Show>
            </Show>
        </div>
    );
}

export const SkillManager = (): JSX.Element => {
    const m = createManagerState();
    const listView = <SkillList m={m} />;
    const detailView = <SkillDetail m={m} />;

    return (
        <PrimitiveListDetail
            showDetail={inDetail(m)}
            backLabel="Skills"
            onBack={() => goBack(m)}
            list={listView}
            detail={detailView}
        />
    );
};

SkillManager.displayName = "SkillManager";
