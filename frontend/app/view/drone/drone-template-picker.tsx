// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// "New from a template" in the Drone pane: the Drone templates (built in, a catalog's, the person's own), each one
// approved before first use exactly as a job template is (what it writes, its credentials, its caps), its settings,
// and a drone made from it that opens on the canvas. It is a panel inside the pane, never a window modal. Making a
// drone starts nothing.
// reactive-coverage: one-shot the panel is opened afresh each time, reads the templates when it opens, and reads them again from the answer to each approval; the templates change only by a person's action in this panel or outside AgentMux

import { createSignal, For, onMount, Show, type JSX } from "solid-js";

import { Button, Field, Pill, TextInput, type PillTone } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { paramForm, paramsFromForm, type ParamValue } from "@/app/view/warden-jobs/job-draft";
import { JobParamsForm } from "@/app/view/warden-jobs/job-params-form";
import { TemplateApproval } from "@/app/view/warden-jobs/job-template-approval";
import type { JobTemplateInfo } from "@/types/rpc/JobTemplateInfo";

import type { DroneViewModel } from "./drone-model";

/** The Drone templates among a list of templates, in the order given. */
export function droneTemplatesIn(all: readonly JobTemplateInfo[]): JobTemplateInfo[] {
    return all.filter((info) => info.manifest?.kind === "drone");
}

const STATE_PILL: Record<string, { label: () => string; tone: PillTone }> = {
    approved: { label: () => t("drone.templates.approved"), tone: "ok" },
    needs_approval: { label: () => t("drone.templates.needsApproval"), tone: "warn" },
    changed: { label: () => t("drone.templates.changed"), tone: "warn" },
    invalid: { label: () => t("drone.templates.invalid"), tone: "error" },
};

/** A chosen template's name and settings, and the button that makes the drone. */
const TemplateForm = (p: {
    info: JobTemplateInfo;
    name: string;
    values: Record<string, ParamValue>;
    errors: Record<string, string>;
    busy: boolean;
    onName: (name: string) => void;
    onChange: (key: string, value: ParamValue) => void;
    onMake: () => void;
    onBack: () => void;
}): JSX.Element => (
    <div class="drone-templates-form">
        <Field label={t("drone.templates.name")} layout="stacked">
            <TextInput value={p.name} maxLength={200} onInput={(e) => p.onName(e.currentTarget.value)} />
        </Field>
        <Show when={p.info.manifest}>
            {(manifest) => (
                <JobParamsForm
                    manifest={manifest()}
                    values={p.values}
                    errors={p.errors}
                    onChange={p.onChange}
                    vault={null}
                />
            )}
        </Show>
        <div class="drone-templates-actions">
            <Button tone="accent" busy={p.busy} onClick={() => p.onMake()}>
                {t("drone.templates.make")}
            </Button>
            <Button tone="quiet" disabled={p.busy} onClick={() => p.onBack()}>
                {t("drone.templates.back")}
            </Button>
        </div>
    </div>
);

/** The templates, each with its state and what can be done with it: use it, or review it first. */
const TemplateList = (p: {
    templates: JobTemplateInfo[];
    loading: boolean;
    onUse: (info: JobTemplateInfo) => void;
    onReview: (info: JobTemplateInfo) => void;
}): JSX.Element => (
    <Show when={!p.loading} fallback={<div class="drone-templates-note">{t("drone.templates.loading")}</div>}>
        <Show
            when={p.templates.length > 0}
            fallback={<div class="drone-templates-note">{t("drone.templates.none")}</div>}
        >
            <ul class="drone-templates-list">
                <For each={p.templates}>
                    {(info) => {
                        const pill = STATE_PILL[info.state] ?? STATE_PILL.invalid;
                        return (
                            <li class="drone-templates-item">
                                <div class="drone-templates-item-head">
                                    <strong>{info.manifest?.name ?? info.id}</strong>
                                    <Pill tone={pill.tone}>{pill.label()}</Pill>
                                </div>
                                <div class="drone-templates-note">{info.manifest?.description ?? info.error}</div>
                                <div class="drone-templates-actions">
                                    <Show when={info.state === "approved"}>
                                        <Button density="compact" onClick={() => p.onUse(info)}>
                                            {t("drone.templates.use")}
                                        </Button>
                                    </Show>
                                    <Show when={info.state === "needs_approval" || info.state === "changed"}>
                                        <Button density="compact" onClick={() => p.onReview(info)}>
                                            {t("drone.templates.review")}
                                        </Button>
                                    </Show>
                                </div>
                            </li>
                        );
                    }}
                </For>
            </ul>
        </Show>
    </Show>
);

/** Run `work` with the panel busy and no old refusal; a refusal from it is shown in the panel, not thrown. */
async function guarded(
    setBusy: (busy: boolean) => void,
    setFailure: (message: string | null) => void,
    work: () => Promise<void>
): Promise<void> {
    setBusy(true);
    setFailure(null);
    try {
        await work();
    } catch (e) {
        setFailure(String(e));
    } finally {
        setBusy(false);
    }
}

/** The panel's state and what its buttons do: list, approve, choose, make. */
function usePicker(p: { model: DroneViewModel; onClose: () => void }) {
    const [templates, setTemplates] = createSignal<JobTemplateInfo[]>([]);
    const [loading, setLoading] = createSignal(true);
    const [failure, setFailure] = createSignal<string | null>(null);
    const [approving, setApproving] = createSignal<JobTemplateInfo | null>(null);
    const [chosen, setChosen] = createSignal<JobTemplateInfo | null>(null);
    const [values, setValues] = createSignal<Record<string, ParamValue>>({});
    const [errors, setErrors] = createSignal<Record<string, string>>({});
    const [name, setName] = createSignal("");
    const [busy, setBusy] = createSignal(false);

    onMount(() => {
        void (async () => {
            try {
                setTemplates(droneTemplatesIn((await RpcApi.JobsTemplatesListCommand(TabRpcClient)).templates));
            } catch (e) {
                setFailure(String(e));
            } finally {
                setLoading(false);
            }
        })();
    });

    const choose = (info: JobTemplateInfo) => {
        setChosen(info);
        setName(info.manifest?.name ?? info.id);
        setValues(info.manifest ? paramForm(info.manifest) : {});
        setErrors({});
        setFailure(null);
    };

    const approve = async () => {
        const info = approving();
        if (!info) return;
        await guarded(setBusy, setFailure, async () => {
            const res = await RpcApi.JobsTemplatesApproveCommand(TabRpcClient, {
                id: info.id,
                hash: info.hash,
                signer: info.signature.fingerprint ?? "",
            });
            setTemplates(droneTemplatesIn(res.templates));
            setApproving(null);
        });
    };

    const make = async () => {
        const info = chosen();
        const manifest = info?.manifest;
        if (!info || !manifest) return;
        const { params, errors: bad } = paramsFromForm(manifest, values());
        setErrors(bad);
        if (Object.keys(bad).length > 0) return;
        await guarded(setBusy, setFailure, async () => {
            const made = await RpcApi.JobsTemplatesMakeDroneCommand(TabRpcClient, {
                id: info.id,
                version: manifest.version,
                params,
                name: name().trim() || undefined,
            });
            await p.model.openDrone(made.drone.id);
            void p.model.refreshList();
            p.onClose();
        });
    };

    const cancelApproval = () => {
        setApproving(null);
        setFailure(null);
    };
    const back = () => {
        setChosen(null);
        setFailure(null);
    };

    return {
        templates,
        loading,
        failure,
        approving,
        setApproving,
        chosen,
        values,
        setValues,
        errors,
        name,
        setName,
        busy,
        choose,
        approve,
        make,
        cancelApproval,
        back,
    };
}

export const DroneTemplatePicker = (p: { model: DroneViewModel; onClose: () => void }): JSX.Element => {
    const {
        templates,
        loading,
        failure,
        approving,
        setApproving,
        chosen,
        values,
        setValues,
        errors,
        name,
        setName,
        busy,
        choose,
        approve,
        make,
        cancelApproval,
        back,
    } = usePicker(p);
    return (
        <section class="drone-templates" aria-label={t("drone.templates.title")}>
            <div class="drone-templates-head">
                <h3 class="drone-templates-title">{t("drone.templates.title")}</h3>
                <Button tone="quiet" density="compact" disabled={busy()} onClick={() => p.onClose()}>
                    {t("drone.templates.close")}
                </Button>
            </div>
            <Show when={failure()}>
                {(message) => (
                    // A refusal is brought into view: the person may have pressed a button at the bottom of a scrolled panel.
                    <div
                        class="drone-templates-error"
                        role="alert"
                        ref={(el) => queueMicrotask(() => el.scrollIntoView?.({ block: "nearest" }))}
                    >
                        {message()}
                    </div>
                )}
            </Show>
            <Show when={approving()}>
                {(info) => (
                    <div data-operator-only="">
                        <TemplateApproval
                            info={info()}
                            busy={busy()}
                            error={null}
                            onApprove={() => void approve()}
                            onCancel={cancelApproval}
                        />
                    </div>
                )}
            </Show>
            <Show when={!approving() && chosen()}>
                {(info) => (
                    <TemplateForm
                        info={info()}
                        name={name()}
                        values={values()}
                        errors={errors()}
                        busy={busy()}
                        onName={setName}
                        onChange={(key, value) => setValues({ ...values(), [key]: value })}
                        onMake={() => void make()}
                        onBack={back}
                    />
                )}
            </Show>
            <Show when={!approving() && !chosen()}>
                <TemplateList templates={templates()} loading={loading()} onUse={choose} onReview={setApproving} />
            </Show>
        </section>
    );
};
