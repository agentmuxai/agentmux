// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Jobs section's editor: New job (blank, or from a template), Edit and Clone
// (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §5.2, §5.4). It builds a job definition and
// sends it with `jobs.save`, which only the operator's own window may do. Dry run shows what the
// definition would do and starts nothing. A template must be approved before a job is made from it.

import { createMemo, createSignal, For, onCleanup, onMount, Show, type Accessor, type JSX, type Setter } from "solid-js";
import { createStore, reconcile, type SetStoreFunction } from "solid-js/store";

import {
    Button,
    Field,
    Pill,
    SegmentedControl,
    Select,
    TextInput,
    type PillTone,
    type SelectOption,
} from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { Rich } from "@/app/i18n/rich";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { sortByLastUsed } from "@/app/view/agent/agent-recency";
import { useAgentList } from "@/app/view/agent/use-agent-list";
import { refreshVault, vaultAvailable, vaultEntries } from "@/app/view/vault/vault-store";
import type { CapDefaults } from "@/types/rpc/CapDefaults";
import type { JobTemplateInfo } from "@/types/rpc/JobTemplateInfo";
import type { JobsDryRunResult } from "@/types/rpc/JobsDryRunResult";
import type { JobsSaveResult } from "@/types/rpc/JobsSaveResult";
import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";

import {
    blankDraft,
    buildDefinition,
    draftFromDefinition,
    draftFromTemplate,
    type JobDraft,
    type ParamValue,
} from "./job-draft";
import { MakeDroneForm } from "./job-drone-template";
import { JobDryRun } from "./job-dry-run";
import { PolicySection, WhatSection, WhenSection, withCurrent } from "./job-editor-fields";
import { JobParamsForm } from "./job-params-form";
import { TemplateApproval } from "./job-template-approval";
import type { JobsModel } from "./jobs-model";

import "./job-editor.scss";

export type EditorStart = { mode: "new" } | { mode: "edit" | "clone"; jobId: string };

const STATE_TONE: Record<string, PillTone> = {
    approved: "ok",
    needs_approval: "warn",
    changed: "warn",
    invalid: "error",
};

/** A template's state as a pill: its words follow the language. Anything unknown reads as invalid. */
function statePill(state: string): { label: string; tone: PillTone } {
    const known = state in STATE_TONE ? state : "invalid";
    const key = known === "needs_approval" ? "needsApproval" : known;
    return { label: t(`jobs.editor.state.${key}`), tone: STATE_TONE[known] };
}

interface JobEditorProps {
    start: EditorStart;
    model: JobsModel;
    defaults: CapDefaults | undefined;
    onClose: () => void;
}

/** What the editor's parts share: its fields, the signals around them and the Vault and agent lists. */
interface EditorState {
    props: JobEditorProps;
    draft: JobDraft;
    setDraft: SetStoreFunction<JobDraft>;
    templates: Accessor<JobTemplateInfo[]>;
    setTemplates: Setter<JobTemplateInfo[]>;
    loading: Accessor<boolean>;
    setLoading: Setter<boolean>;
    source: Accessor<"blank" | "template">;
    setSource: Setter<"blank" | "template">;
    chosen: Accessor<boolean>;
    setChosen: Setter<boolean>;
    approving: Accessor<JobTemplateInfo | null>;
    setApproving: Setter<JobTemplateInfo | null>;
    busy: Accessor<"save" | "dry" | "approve" | null>;
    setBusy: Setter<"save" | "dry" | "approve" | null>;
    errors: Accessor<string[]>;
    setErrors: Setter<string[]>;
    paramErrors: Accessor<Record<string, string>>;
    setParamErrors: Setter<Record<string, string>>;
    failure: Accessor<string | null>;
    setFailure: Setter<string | null>;
    dry: Accessor<JobsDryRunResult | null>;
    setDry: Setter<JobsDryRunResult | null>;
    confirm: Accessor<JobsSaveResult | null>;
    setConfirm: Setter<JobsSaveResult | null>;
    manifest: Accessor<JobTemplateInfo["manifest"]>;
    agentOptions: Accessor<SelectOption[]>;
    droneOptions: Accessor<SelectOption[]>;
    nameOf: (id: string) => string;
    vault: () => readonly VaultEntryInfo[] | null;
}

/** The drones a job can run, by name. None when they can't be listed. */
async function loadDroneOptions(): Promise<SelectOption[]> {
    try {
        const list = await RpcApi.ListDronesCommand(TabRpcClient, {});
        return list.map((d) => ({ value: d.id, label: d.name || d.id }));
    } catch {
        return [];
    }
}

function createEditorState(props: JobEditorProps): EditorState {
    const [draft, setDraft] = createStore<JobDraft>(blankDraft());
    const [templates, setTemplates] = createSignal<JobTemplateInfo[]>([]);
    const [loading, setLoading] = createSignal(props.start.mode !== "new");
    const [source, setSource] = createSignal<"blank" | "template">("blank");
    const [chosen, setChosen] = createSignal(false);
    const [approving, setApproving] = createSignal<JobTemplateInfo | null>(null);
    const [busy, setBusy] = createSignal<"save" | "dry" | "approve" | null>(null);
    const [errors, setErrors] = createSignal<string[]>([]);
    const [paramErrors, setParamErrors] = createSignal<Record<string, string>>({});
    const [failure, setFailure] = createSignal<string | null>(null);
    const [dry, setDry] = createSignal<JobsDryRunResult | null>(null);
    const [confirm, setConfirm] = createSignal<JobsSaveResult | null>(null);
    const [agents] = useAgentList();

    const manifest = createMemo(() => templates().find((tpl) => tpl.id === draft.template?.id)?.manifest ?? null);
    const agentOptions = createMemo<SelectOption[]>(() =>
        sortByLastUsed(
            (agents() ?? []).filter((a) => !a.is_seeded),
            (a) => a.name
        ).map((a) => ({ value: a.id, label: a.name }))
    );
    const [droneOptions, setDroneOptions] = createSignal<SelectOption[]>([]);
    // The list follows drones made or removed while the editor is open.
    const reloadDrones = () => void loadDroneOptions().then(setDroneOptions);
    onMount(reloadDrones);
    onCleanup(onMuxEvent(WpsEvent.DronesChanged, reloadDrones));
    const nameOf = (id: string) => agentOptions().find((o) => o.value === id)?.label ?? id;
    const vault = () => (vaultAvailable() ? (vaultEntries() ?? []) : null);
    return {
        props,
        draft,
        setDraft,
        templates,
        setTemplates,
        loading,
        setLoading,
        source,
        setSource,
        chosen,
        setChosen,
        approving,
        setApproving,
        busy,
        setBusy,
        errors,
        setErrors,
        paramErrors,
        setParamErrors,
        failure,
        setFailure,
        dry,
        setDry,
        confirm,
        setConfirm,
        manifest,
        agentOptions,
        droneOptions,
        nameOf,
        vault,
    };
}

async function loadEditor(ed: EditorState): Promise<void> {
    if (vaultAvailable()) void refreshVault();
    try {
        ed.setTemplates((await ed.props.model.templates()).templates);
    } catch (e) {
        ed.setFailure(String(e));
    }
    const start = ed.props.start;
    if (start.mode === "new") return;
    try {
        const { definition } = await ed.props.model.get(start.jobId);
        const id = (definition.template as { id?: string } | undefined)?.id;
        const info = ed.templates().find((tpl) => tpl.id === id);
        ed.setDraft(reconcile(draftFromDefinition(definition, info?.manifest ?? null, start.mode)));
        ed.setChosen(true);
    } catch (e) {
        ed.setFailure(String(e));
    } finally {
        ed.setLoading(false);
    }
}

function chooseSource(ed: EditorState, next: "blank" | "template"): void {
    ed.setSource(next);
    ed.setChosen(false);
    ed.setDry(null);
    ed.setDraft(reconcile(blankDraft()));
}

function useTemplate(ed: EditorState, info: JobTemplateInfo): void {
    ed.setDraft(reconcile(draftFromTemplate(info)));
    ed.setChosen(true);
    ed.setDry(null);
}

async function approveTemplate(ed: EditorState): Promise<void> {
    const info = ed.approving();
    if (!info) return;
    ed.setBusy("approve");
    ed.setFailure(null);
    try {
        const res = await ed.props.model.approveTemplate({
            id: info.id,
            hash: info.hash,
            signer: info.signature.fingerprint ?? "",
        });
        ed.setTemplates(res.templates);
        ed.setApproving(null);
    } catch (e) {
        ed.setFailure(String(e));
    } finally {
        ed.setBusy(null);
    }
}

/** The definition for the draft, or null (with the reasons shown) when it can't be sent yet. */
function builtDefinition(ed: EditorState) {
    const b = buildDefinition(ed.draft, ed.manifest());
    ed.setErrors(b.errors);
    ed.setParamErrors(b.paramErrors);
    return b.errors.length > 0 ? null : b.definition;
}

async function saveJob(ed: EditorState, confirm: string | null = null): Promise<void> {
    ed.setFailure(null);
    const definition = builtDefinition(ed);
    if (!definition) return;
    ed.setBusy("save");
    try {
        const res = await ed.props.model.save({
            ...(ed.draft.id ? { job_id: ed.draft.id } : {}),
            definition,
            ...(confirm === "no_cost" ? { confirm_no_cost: true } : {}),
            ...(confirm === "turn_on" ? { confirm_turn_on: true } : {}),
        });
        if (res.needs_confirmation) ed.setConfirm(res);
        else ed.props.onClose();
    } catch (e) {
        ed.setFailure(String(e));
    } finally {
        ed.setBusy(null);
    }
}

async function dryRunJob(ed: EditorState): Promise<void> {
    ed.setFailure(null);
    const definition = builtDefinition(ed);
    if (!definition) return;
    ed.setBusy("dry");
    try {
        ed.setDry(await ed.props.model.dryRun({ ...(ed.draft.id ? { job_id: ed.draft.id } : {}), definition }));
    } catch (e) {
        ed.setFailure(String(e));
    } finally {
        ed.setBusy(null);
    }
}

const editorTitle = (ed: EditorState) =>
    ed.props.start.mode === "edit"
        ? t("jobs.editor.edit", { name: ed.draft.name || t("jobs.editor.jobFallback") })
        : ed.props.start.mode === "clone"
          ? t("jobs.editor.clone")
          : t("jobs.editor.new");
const showForm = (ed: EditorState) =>
    !ed.loading() && (ed.props.start.mode !== "new" || ed.source() === "blank" || ed.chosen());

function TemplateItem(p: { ed: EditorState; t: JobTemplateInfo }): JSX.Element {
    const pill = statePill(p.t.state);
    const isDrone = p.t.manifest?.kind === "drone";
    // A Drone template makes a drone (its graph), not a job: its settings are asked for here.
    const [making, setMaking] = createSignal(false);
    return (
        <li class="warden-jobs-template">
            <div class="warden-jobs-template-head">
                <strong>{p.t.manifest?.name ?? p.t.id}</strong>
                <span class="warden-manager-dim">{p.t.manifest?.version}</span>
                <Pill tone={pill.tone}>{pill.label}</Pill>
                <Show when={isDrone}>
                    <Pill>{t("jobs.droneTemplate.pill")}</Pill>
                </Show>
                <Show when={p.t.source === "builtin"}>
                    <Pill>AgentMux</Pill>
                </Show>
            </div>
            <div class="warden-manager-dim">{p.t.manifest?.description ?? p.t.error}</div>
            <Show when={making()}>
                <MakeDroneForm info={p.t} onDone={() => p.ed.props.onClose()} onBack={() => setMaking(false)} />
            </Show>
            <div class="warden-jobs-actions">
                <Show when={p.t.state === "approved" && !making()}>
                    <Show
                        when={isDrone}
                        fallback={
                            <Button density="compact" icon="clone" onClick={() => useTemplate(p.ed, p.t)}>
                                {t("jobs.editor.cloneToJob")}
                            </Button>
                        }
                    >
                        <Button density="compact" icon="diagram-project" onClick={() => setMaking(true)}>
                            {t("jobs.droneTemplate.make")}
                        </Button>
                    </Show>
                </Show>
                <Show when={p.t.state === "needs_approval" || p.t.state === "changed"}>
                    <Button density="compact" icon="shield-halved" onClick={() => p.ed.setApproving(p.t)}>
                        {t("jobs.editor.reviewApprove")}
                    </Button>
                </Show>
            </div>
        </li>
    );
}

function StartFrom(p: { ed: EditorState }): JSX.Element {
    return (
        <>
            <Field label={t("jobs.editor.startFrom")} layout="stacked">
                <SegmentedControl
                    options={[
                        { value: "blank", label: t("jobs.editor.blank") },
                        { value: "template", label: t("jobs.editor.template") },
                    ]}
                    value={p.ed.source()}
                    onChange={(next) => chooseSource(p.ed, next)}
                />
            </Field>
            <Show when={p.ed.source() === "template" && !p.ed.chosen()}>
                <Show
                    when={p.ed.templates().length > 0}
                    fallback={<div class="warden-section-stub">{t("jobs.editor.noTemplates")}</div>}
                >
                    <ul class="warden-jobs-templates">
                        <For each={p.ed.templates()}>{(tpl) => <TemplateItem ed={p.ed} t={tpl} />}</For>
                    </ul>
                </Show>
            </Show>
        </>
    );
}

function JobForm(p: { ed: EditorState }): JSX.Element {
    const ed = p.ed;
    return (
        <>
            <Show when={ed.draft.template}>
                {(tpl) => (
                    <div class="warden-manager-dim">
                        <Rich
                            k="jobs.editor.madeFrom"
                            params={{
                                name: ed.manifest()?.name ?? tpl().id,
                                version: tpl().version || ed.manifest()?.version || "",
                            }}
                        />
                    </div>
                )}
            </Show>
            <div class="warden-jobs-form">
                <div class="warden-jobs-grid">
                    <Field label={t("jobs.editor.name")} layout="stacked">
                        <TextInput
                            value={ed.draft.name}
                            onInput={(e) => ed.setDraft("name", e.currentTarget.value)}
                            maxLength={200}
                        />
                    </Field>
                    <Field label={t("jobs.editor.owner")} layout="stacked" description={t("jobs.editor.ownerHint")}>
                        <Select
                            value={ed.draft.owner}
                            onChange={(v) => ed.setDraft("owner", v)}
                            options={withCurrent(ed.agentOptions(), ed.draft.owner, t("jobs.editor.noAgent"))}
                        />
                    </Field>
                </div>
                <Show when={ed.draft.template && ed.manifest()}>
                    {(m) => (
                        <JobParamsForm
                            manifest={m()}
                            values={ed.draft.params}
                            errors={ed.paramErrors()}
                            vault={ed.vault()}
                            onChange={(name: string, value: ParamValue) => ed.setDraft("params", name, value)}
                        />
                    )}
                </Show>
                <Show when={ed.draft.template && !ed.manifest() && !ed.loading()}>
                    <div class="warden-manager-error">
                        {t("jobs.editor.templateGone")}
                    </div>
                </Show>
                <WhenSection draft={ed.draft} set={ed.setDraft} />
                <WhatSection
                    draft={ed.draft}
                    set={ed.setDraft}
                    agents={ed.agentOptions()}
                    drones={ed.droneOptions()}
                    templated={ed.draft.template != null}
                />
                <PolicySection draft={ed.draft} set={ed.setDraft} defaults={ed.props.defaults} />
            </div>
        </>
    );
}

function ConfirmNote(p: { ed: EditorState; result: JobsSaveResult }): JSX.Element {
    return (
        <div class="warden-jobs-note" role="alert">
            {p.result.error}
            <Show when={p.result.lines.length > 0}>
                <ul class="warden-jobs-turn-on">
                    <For each={p.result.lines}>{(line) => <li>{line}</li>}</For>
                </ul>
            </Show>
            <div class="warden-jobs-actions">
                <Button
                    tone="danger"
                    icon="check"
                    busy={p.ed.busy() === "save"}
                    onClick={() => void saveJob(p.ed, p.result.needs_confirmation)}
                >
                    {p.result.needs_confirmation === "turn_on" ? t("jobs.turnOn.confirm") : t("jobs.editor.confirmSave")}
                </Button>
                <Button tone="quiet" onClick={() => p.ed.setConfirm(null)}>
                    {t("jobs.editor.notNow")}
                </Button>
            </div>
        </div>
    );
}

function EditorActions(p: { ed: EditorState }): JSX.Element {
    const ed = p.ed;
    return (
        <div class="warden-jobs-actions">
            <Button
                tone="accent"
                icon="floppy-disk"
                busy={ed.busy() === "save"}
                disabled={!showForm(ed) || ed.busy() != null}
                onClick={() => void saveJob(ed)}
            >
                {ed.draft.id ? t("jobs.editor.saveChanges") : t("jobs.editor.create")}
            </Button>
            <Button
                icon="vial"
                busy={ed.busy() === "dry"}
                disabled={!showForm(ed) || ed.busy() != null}
                onClick={() => void dryRunJob(ed)}
            >
                {t("jobs.editor.dryRun")}
            </Button>
            <Button tone="quiet" onClick={() => ed.props.onClose()}>
                {t("jobs.editor.cancel")}
            </Button>
        </div>
    );
}

export function JobEditor(props: JobEditorProps): JSX.Element {
    const ed = createEditorState(props);
    onMount(() => loadEditor(ed));

    return (
        <section class="warden-jobs-editor" aria-label={editorTitle(ed)}>
            <h3 class="warden-jobs-heading">{editorTitle(ed)}</h3>
            <Show when={ed.approving()}>
                {(info) => (
                    <TemplateApproval
                        info={info()}
                        busy={ed.busy() === "approve"}
                        error={ed.failure()}
                        onApprove={() => void approveTemplate(ed)}
                        onCancel={() => ed.setApproving(null)}
                    />
                )}
            </Show>
            <Show when={!ed.approving()}>
                <Show when={props.start.mode === "new"}>
                    <StartFrom ed={ed} />
                </Show>
                <Show when={ed.loading()}>
                    <div class="warden-section-stub">{t("jobs.editor.loading")}</div>
                </Show>
                <Show when={showForm(ed)}>
                    <JobForm ed={ed} />
                </Show>
                <Show when={ed.errors().length > 0}>
                    <ul class="warden-manager-error warden-jobs-problems" role="alert">
                        <For each={ed.errors()}>{(e) => <li>{e}</li>}</For>
                    </ul>
                </Show>
                <Show when={ed.failure()}>
                    <div class="warden-manager-error" role="alert">
                        {ed.failure()}
                    </div>
                </Show>
                <Show when={ed.confirm()}>{(c) => <ConfirmNote ed={ed} result={c()} />}</Show>
                <Show when={ed.dry()}>{(d) => <JobDryRun result={d()} vault={ed.vault()} nameOf={ed.nameOf} />}</Show>
                <EditorActions ed={ed} />
            </Show>
        </section>
    );
}
