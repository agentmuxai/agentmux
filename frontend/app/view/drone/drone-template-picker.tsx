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

export const DroneTemplatePicker = (p: { model: DroneViewModel; onClose: () => void }): JSX.Element => {
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
        setBusy(true);
        setFailure(null);
        try {
            const res = await RpcApi.JobsTemplatesApproveCommand(TabRpcClient, {
                id: info.id,
                hash: info.hash,
                signer: info.signature.fingerprint ?? "",
            });
            setTemplates(droneTemplatesIn(res.templates));
            setApproving(null);
        } catch (e) {
            setFailure(String(e));
        } finally {
            setBusy(false);
        }
    };

    const make = async () => {
        const info = chosen();
        const manifest = info?.manifest;
        if (!info || !manifest) return;
        const { params, errors: bad } = paramsFromForm(manifest, values());
        setErrors(bad);
        if (Object.keys(bad).length > 0) return;
        setBusy(true);
        setFailure(null);
        try {
            const made = await RpcApi.JobsTemplatesMakeDroneCommand(TabRpcClient, {
                id: info.id,
                version: manifest.version,
                params,
                name: name().trim() || undefined,
            });
            await p.model.openDrone(made.drone.id);
            void p.model.refreshList();
            p.onClose();
        } catch (e) {
            setFailure(String(e));
        } finally {
            setBusy(false);
        }
    };

    return (
        <section class="drone-templates" aria-label={t("drone.templates.title")}>
            <div class="drone-templates-head">
                <h3 class="drone-templates-title">{t("drone.templates.title")}</h3>
                <Button tone="quiet" density="compact" disabled={busy()} onClick={() => p.onClose()}>
                    {t("drone.templates.close")}
                </Button>
            </div>
            <Show when={failure()}>
                <div class="drone-templates-error" role="alert">
                    {failure()}
                </div>
            </Show>
            <Show when={approving()}>
                {(info) => (
                    <TemplateApproval
                        info={info()}
                        busy={busy()}
                        error={null}
                        onApprove={() => void approve()}
                        onCancel={() => setApproving(null)}
                    />
                )}
            </Show>
            <Show when={!approving() && chosen()}>
                {(info) => (
                    <div class="drone-templates-form">
                        <Field label={t("drone.templates.name")} layout="stacked">
                            <TextInput value={name()} maxLength={200} onInput={(e) => setName(e.currentTarget.value)} />
                        </Field>
                        <Show when={info().manifest}>
                            {(manifest) => (
                                <JobParamsForm
                                    manifest={manifest()}
                                    values={values()}
                                    errors={errors()}
                                    onChange={(key, value) => setValues({ ...values(), [key]: value })}
                                    vault={null}
                                />
                            )}
                        </Show>
                        <div class="drone-templates-actions">
                            <Button tone="accent" busy={busy()} onClick={() => void make()}>
                                {t("drone.templates.make")}
                            </Button>
                            <Button tone="quiet" disabled={busy()} onClick={() => setChosen(null)}>
                                {t("drone.templates.back")}
                            </Button>
                        </div>
                    </div>
                )}
            </Show>
            <Show when={!approving() && !chosen()}>
                <Show when={!loading()} fallback={<div class="drone-templates-note">{t("drone.templates.loading")}</div>}>
                    <Show when={templates().length > 0} fallback={<div class="drone-templates-note">{t("drone.templates.none")}</div>}>
                        <ul class="drone-templates-list">
                            <For each={templates()}>
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
                                                    <Button density="compact" onClick={() => choose(info)}>
                                                        {t("drone.templates.use")}
                                                    </Button>
                                                </Show>
                                                <Show when={info.state === "needs_approval" || info.state === "changed"}>
                                                    <Button density="compact" onClick={() => setApproving(info)}>
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
            </Show>
        </section>
    );
};
