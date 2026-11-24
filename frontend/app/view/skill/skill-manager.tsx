// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * SkillManager — the Memory → Skills section body. Context-free
 * list/create/edit/delete over the global Skill catalog (skill.catalog.*
 * MuxAPI via SkillCatalogModel). Every row here is global; an agent's own
 * private skills are not shown here. A skill's details also say who gets it
 * without a bind (every agent, new agents).
 */

// reactive-coverage: one-shot the installed application changes only when the person installs or removes it outside AgentMux, so its version is read when Memory opens and again whenever the skill list changes (an install, an edit, a delete)

import { createResource, createSignal, onCleanup, For, Show, type Accessor, type JSX, type Setter } from "solid-js";
import type { AppSkillHere } from "@/types/rpc/AppSkillHere";
import type { SkillCatalogItem } from "@/types/rpc/SkillCatalogItem";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { t } from "@/app/i18n/i18n";
import { Markdown } from "@/app/element/markdown";
import { PrimitiveListDetail } from "@/app/element/primitive-list-detail";
import { showTextInputContextMenu } from "@/app/store/contextmenu";
import { SkillCatalogModel, type SkillDraft } from "./skill-model";
import { SkillImportModel } from "./skill-import-model";
import { SkillImportPanel, SkillImportResult } from "./skill-import";
import { SkillMatrix } from "./skill-matrix";
import { AppFilterRow, SkillAppDetail } from "./skill-app";
import { SkillBrowse } from "./skill-browse";
import { appHereFact, appPlatform, appStatus, filterSkills, firstRoute, type AppFilter } from "./skill-app-model";
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
            <ul class="agent-primitive-modal-bound" aria-label={t("skill.manager.boundAria")}>
                <For each={bound()}>
                    {(agent) => (
                        <li class="agent-primitive-modal-bound-agent">
                            <span>{agent.name}</span>
                            <Button
                                density="compact"
                                aria-label={t("skill.manager.unbindAria", { name: agent.name })}
                                onClick={() => void props.model.unbindFromAgent(props.skill.id, agent.id)}
                            >
                                {t("skill.manager.unbind")}
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
        <div class="agent-primitive-modal-reach" aria-label={t("skill.manager.reachAria")}>
            <div class="agent-primitive-modal-reach-row">
                <Switch
                    checked={props.skill.all_agents}
                    ariaLabel={t("skill.manager.everyAgentAria")}
                    onChange={(on) => void props.model.setReach(props.skill.id, on, props.skill.new_agents || on)}
                />
                <span>{t("skill.manager.everyAgentGets")}</span>
            </div>
            <div class="agent-primitive-modal-reach-row">
                <Switch
                    checked={props.skill.all_agents || props.skill.new_agents}
                    disabled={props.skill.all_agents}
                    ariaLabel={t("skill.manager.newAgentsAria")}
                    onChange={(on) => void props.model.setReach(props.skill.id, false, on)}
                />
                <span>{t("skill.manager.newAgentsGet")}</span>
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
    showBrowse: Accessor<boolean>;
    setShowBrowse: Setter<boolean>;
    filter: Accessor<AppFilter>;
    setFilter: Setter<AppFilter>;
    /** Where each installed App skill's application is on this computer, by skill id; empty until srv answers. */
    appHere: Accessor<Record<string, AppSkillHere>>;
}

function createManagerState(): ManagerState {
    const model = new SkillCatalogModel();
    onCleanup(() => model.dispose());

    // Single-pane — see docs/specs/SPEC_ARMORY_RESPONSIVE_SINGLE_PANE_LAYOUT_2026_07_15.md.
    // The import panel, or an imported skill's "check for changes" (`updateOf` set).
    const [importing, setImporting] = createSignal<Importing | null>(null);
    const [showMatrix, setShowMatrix] = createSignal(false);
    const [showBrowse, setShowBrowse] = createSignal(false);
    const [filter, setFilter] = createSignal<AppFilter>("all");
    // Read again when the catalog changes (an install, an edit, a delete); a lookup that fails leaves the cards as they were.
    const [here] = createResource(
        () => model.revisionAtom(),
        async () => Object.fromEntries((await RpcApi.AppSkillsHereCommand(TabRpcClient, {})).items.map((h) => [h.skill_id, h])),
    );
    const appHere = () => (here.error ? {} : (here() ?? {}));
    return { model, importing, setImporting, showMatrix, setShowMatrix, showBrowse, setShowBrowse, filter, setFilter, appHere };
}

const inDetail = (m: ManagerState) =>
    m.model.selectedIdAtom() !== null || m.model.draftAtom() !== null || m.importing() !== null || m.showMatrix() || m.showBrowse();

function importedSkills(m: ManagerState, ids: string[]): void {
    m.setImporting(null);
    void m.model.refresh();
    if (ids[0]) m.model.setSelectedId(ids[0]);
}

function startImport(m: ManagerState): void {
    m.setShowMatrix(false);
    m.setShowBrowse(false);
    m.model.setSelectedId(null);
    m.model.setDraft(null);
    m.setImporting({ model: new SkillImportModel((ids) => importedSkills(m, ids)) });
}

function startCheck(m: ManagerState, skill: SkillCatalogItem): void {
    m.setShowBrowse(false);
    const check = new SkillImportModel((ids) => importedSkills(m, ids));
    m.setImporting({ model: check, updateOf: skill });
    void check.check(skill.id);
}

function goBack(m: ManagerState): void {
    m.setShowMatrix(false);
    m.setShowBrowse(false);
    m.setImporting(null);
    m.model.setError(null);
    m.model.setSelectedId(null);
    m.model.setDraft(null);
}

/** Who gets a skill, as the short facts a card shows: every agent, the new ones, or how many it is bound to. */
function reachFacts(skill: SkillCatalogItem): string[] {
    if (skill.all_agents) return [t("skill.manager.everyAgent")];
    const bound = t("skill.manager.agents", { count: skill.bound_count });
    if (!skill.new_agents) return [bound];
    return skill.bound_count > 0 ? [t("skill.manager.newAgents"), bound] : [t("skill.manager.newAgents")];
}

function SkillCard(p: { m: ManagerState; skill: SkillCatalogItem }): JSX.Element {
    const app = () => p.skill.app;
    const here = () => p.m.appHere()[p.skill.id];
    const status = () => (app() ? appStatus(app()!, appPlatform(), here()) : null);
    const hereFact = () => (app() ? appHereFact(app()!, here(), false) : null);
    // An App skill's card leads with how well it is tested and its route; where it came from is in its detail.
    const facts = () => [
        ...(app() ? [<span>{status()!.label}</span>] : []),
        ...(hereFact() ? [<span>{hereFact()}</span>] : []),
        ...(app() && firstRoute(app()!) ? [<span>{t("skill.manager.route", { route: firstRoute(app()!) ?? "" })}</span>] : []),
        ...reachFacts(p.skill).map((fact) => <span>{fact}</span>),
        ...(p.skill.source && !app() ? [<span>{t("skill.manager.importedFrom", { location: p.skill.source.location })}</span>] : []),
    ];
    return (
        <Card
            class="skill-card"
            icon={<i class="fa-solid fa-wand-magic-sparkles" />}
            title={p.skill.name}
            subtitle={
                app() ? (
                    <>
                        <Pill tone="accent">{t("skill.manager.appSkill")}</Pill> {app()!.vendor}
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
                p.m.setShowBrowse(false);
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
                    {t("skill.manager.newSkill")}
                </Button>
                <Button density="compact" onClick={() => startImport(p.m)}>
                    {t("skill.manager.import")}
                </Button>
                <Button
                    density="compact"
                    onClick={() => {
                        model.setSelectedId(null);
                        model.setDraft(null);
                        p.m.setImporting(null);
                        p.m.setShowMatrix(false);
                        p.m.setShowBrowse(true);
                    }}
                >
                    {t("skill.manager.browse")}
                </Button>
                <Button
                    density="compact"
                    onClick={() => {
                        model.setSelectedId(null);
                        model.setDraft(null);
                        p.m.setImporting(null);
                        p.m.setShowBrowse(false);
                        p.m.setShowMatrix(true);
                    }}
                >
                    {t("skill.manager.matrix")}
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
                        {p.m.filter() === "app" ? t("skill.manager.emptyApp") : t("skill.manager.empty")}
                    </div>
                }
            >
                <div class="primitive-card-body">
                    <CardGrid
                        ariaLabel={t("skill.manager.skillsAria")}
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
            <h3 class="agent-primitive-modal-name">{t("skill.manager.checkTitle", { name: p.skill.name })}</h3>
            <Show when={p.imp.model.errorAtom()}>
                <div class="agent-primitive-modal-error">{p.imp.model.errorAtom()}</div>
            </Show>
            <Show when={p.imp.model.busyAtom() && !p.imp.model.previewAtom()}>
                <p class="agent-primitive-modal-global-note">{t("skill.manager.reading")}</p>
            </Show>
            <SkillImportResult model={p.imp.model} verb={t("skill.manager.update")} />
            <div class="agent-primitive-modal-actions">
                <Button class="agent-primitive-modal-btn" onClick={() => p.m.setImporting(null)}>
                    {t("skill.manager.close")}
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
                    ? t("skill.manager.usedByEvery")
                    : t("skill.manager.usedBy", { count: skill().bound_count })}
            </p>
            <Show when={skill().app}>{(app) => <SkillAppDetail skillId={skill().id} app={app()} here={p.m.appHere()[skill().id]} agents={model.agentsAtom()} />}</Show>
            <SkillReach model={model} skill={skill()} />
            <SkillBoundAgents model={model} skill={skill()} />
            <Show when={skill().source}>
                {(src) => (
                    <>
                        <span class="agent-primitive-modal-field-label">{t("skill.manager.importedFromLabel")}</span>
                        <pre class="agent-primitive-modal-field-value">
                            {src().location}
                            {src().git_ref ? ` (${src().git_ref})` : ""}
                            {src().item_key ? `\n${src().item_key}` : ""}
                        </pre>
                        <div class="agent-primitive-modal-actions">
                            <Button class="agent-primitive-modal-btn" onClick={() => startCheck(p.m, skill())}>
                                {t("skill.manager.checkForChanges")}
                            </Button>
                        </div>
                    </>
                )}
            </Show>
            <Show when={skill().description}>
                <span class="agent-primitive-modal-field-label">{t("skill.manager.description")}</span>
                <pre class="agent-primitive-modal-field-value">{skill().description}</pre>
            </Show>
            <span class="agent-primitive-modal-field-label">{t("skill.manager.format")}</span>
            <pre class="agent-primitive-modal-field-value">
                {skill().skill_type === "agent-skill" ? t("skill.manager.formatAgentSkill") : t("skill.manager.formatSlash")}
            </pre>
            <Show when={skill().skill_type !== "agent-skill" && skill().trigger}>
                <span class="agent-primitive-modal-field-label">{t("skill.manager.trigger")}</span>
                <pre class="agent-primitive-modal-field-value">{skill().trigger}</pre>
            </Show>
            <span class="agent-primitive-modal-field-label">{t("skill.manager.content")}</span>
            <Show when={skill().content} fallback={<pre class="agent-primitive-modal-field-value">{t("skill.manager.none")}</pre>}>
                <div class="agent-primitive-modal-field-value agent-primitive-modal-field-value--markdown">
                    <Markdown text={skill().content} scrollable={false} />
                </div>
            </Show>
            <span class="agent-primitive-modal-field-label">{t("skill.manager.bindToAgent")}</span>
            <div class="agent-primitive-modal-bind-row">
                <select
                    class="agent-primitive-modal-input"
                    value={model.bindAgentIdAtom()}
                    onChange={(e) => model.setBindAgentId(e.currentTarget.value)}
                >
                    <option value="">{t("skill.manager.selectAgent")}</option>
                    <For each={model.agentsAtom()}>{(agent) => <option value={agent.id}>{agent.name}</option>}</For>
                </select>
                <Button
                    class="agent-primitive-modal-btn"
                    disabled={!model.bindAgentIdAtom()}
                    onClick={() => void model.bindToAgent(skill().id, model.bindAgentIdAtom())}
                >
                    {t("skill.manager.bind")}
                </Button>
            </div>
            <div class="agent-primitive-modal-actions">
                <Button
                    tone="danger"
                    class="agent-primitive-modal-btn agent-primitive-modal-btn-danger"
                    onClick={() => void model.deleteSkill(skill().id)}
                >
                    {t("skill.manager.delete")}
                </Button>
                <Button class="agent-primitive-modal-btn" onClick={() => model.startEdit(skill())}>
                    {t("skill.manager.edit")}
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
            <span class="agent-primitive-modal-field-label">{t("skill.manager.nameLabel")}</span>
            <TextInput
                variant="plain"
                class="agent-primitive-modal-input"
                value={draft().name}
                onInput={(e) => model.setDraft({ ...draft(), name: e.currentTarget.value })}
                onContextMenu={showTextInputContextMenu}
                placeholder={t("skill.manager.namePlaceholder")}
                required
                ref={(el) => focusOnOpen(el, { select: true })}
            />
            <span class="agent-primitive-modal-field-label">{t("skill.manager.format")}</span>
            <select
                class="agent-primitive-modal-input"
                value={draft().skill_type || "prompt"}
                onChange={(e) => model.setDraft({ ...draft(), skill_type: e.currentTarget.value })}
            >
                <option value="prompt">{t("skill.manager.typeSlash")}</option>
                <option value="agent-skill">{t("skill.manager.typeAgentSkill")}</option>
            </select>
            <Show when={(draft().skill_type || "prompt") !== "agent-skill"}>
                <span class="agent-primitive-modal-field-label">{t("skill.manager.trigger")}</span>
                <TextInput
                    variant="plain"
                    class="agent-primitive-modal-input"
                    value={draft().trigger}
                    onInput={(e) => model.setDraft({ ...draft(), trigger: e.currentTarget.value })}
                    onContextMenu={showTextInputContextMenu}
                    placeholder={t("skill.manager.triggerPlaceholder")}
                />
            </Show>
            <span class="agent-primitive-modal-field-label">{t("skill.manager.description")}</span>
            <TextInput
                variant="plain"
                class="agent-primitive-modal-input"
                value={draft().description}
                onInput={(e) => model.setDraft({ ...draft(), description: e.currentTarget.value })}
                onContextMenu={showTextInputContextMenu}
            />
            <span class="agent-primitive-modal-field-label">{t("skill.manager.content")}</span>
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
                    {t("skill.manager.cancel")}
                </Button>
                <Button
                    type="submit"
                    tone="accent"
                    class="agent-primitive-modal-btn agent-primitive-modal-btn-primary"
                    disabled={model.savingAtom() || !draft().name.trim()}
                >
                    {model.savingAtom() ? t("skill.manager.saving") : t("skill.manager.save")}
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
            <Show when={p.m.showBrowse() && !p.m.importing()}>
                <SkillBrowse
                    catalog={model}
                    onOpen={(skillId) => {
                        p.m.setShowBrowse(false);
                        model.setSelectedId(skillId);
                    }}
                />
            </Show>
            <Show when={!p.m.importing() && !p.m.showMatrix() && !p.m.showBrowse()}>
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
