// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The operator's approval of a job template: who published it and whether that key is the one pinned,
// when it triggers, what it does, every output and secret it declares and its default caps
// (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §5.4). Approving runs nothing; it lets a job be
// made from exactly this version. Agents can't approve.
//
// A cloud template (§4.3) runs elsewhere, so its prompt shows who owns it, the reporter whose key approving
// pins, and what it is expected to do, instead of a trigger and an action.

import { For, Show, type JSX } from "solid-js";

import { Button, Pill, type PillTone } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import type { CloudExpect } from "@/types/rpc/CloudExpect";
import type { DroneSummary } from "@/types/rpc/DroneSummary";
import type { JobTemplateInfo } from "@/types/rpc/JobTemplateInfo";
import type { JobTemplateManifest } from "@/types/rpc/JobTemplateManifest";
import type { ReporterNote } from "@/types/rpc/ReporterNote";

import { describeCron } from "./jobs-format";

import "./job-editor.scss";

/** Outputs that write outside AgentMux or to someone else (srv's `OUTSIDE_OUTPUTS`). */
const OUTSIDE = new Set(["pull_request", "comment", "message:others", "deploy", "ci_rerun"]);

const writesOutside = (output: string): boolean => OUTSIDE.has(output) || OUTSIDE.has(output.split(":")[0]);

/** The publisher's key, in words and in a tone. */
export function publisherNote(info: JobTemplateInfo): { text: string; tone: PillTone } {
    const sig = info.signature;
    if (info.source === "builtin") return { text: t("jobs.approval.builtin"), tone: "ok" };
    switch (sig.state) {
        case "signed":
            return { text: t("jobs.approval.signed", { publisher: sig.publisher, fingerprint: sig.fingerprint }), tone: "ok" };
        case "signed_new":
            return { text: t("jobs.approval.signedNew", { publisher: sig.publisher, fingerprint: sig.fingerprint }), tone: "warn" };
        case "key_changed":
            return {
                text: t("jobs.approval.keyChanged", { publisher: sig.publisher, pinned: sig.pinned ?? t("jobs.approval.pinnedNone"), fingerprint: sig.fingerprint ?? t("jobs.approval.unsigned") }),
                tone: "error",
            };
        default:
            return { text: t("jobs.approval.unsignedNote", { publisher: sig.publisher }), tone: "warn" };
    }
}

/** What approving does to the reporter's pin, in words and in a tone; `block` when approving would be refused. */
export function reporterNote(note: ReporterNote): { text: string; tone: PillTone; block: boolean } {
    switch (note.state) {
        case "new":
            return {
                text: t("jobs.approval.reporterNew", { fingerprint: note.fingerprint }),
                tone: "warn",
                block: false,
            };
        case "pinned":
            return { text: t("jobs.approval.reporterPinned", { fingerprint: note.fingerprint }), tone: "ok", block: false };
        case "name_taken":
            return {
                text: t("jobs.approval.reporterNameTaken", { name: note.name, fingerprint: note.fingerprint }),
                tone: "error",
                block: true,
            };
        default:
            return {
                text: t("jobs.approval.reporterKeyTaken", { fingerprint: note.fingerprint }),
                tone: "error",
                block: true,
            };
    }
}

function expectWords(expect: CloudExpect | null | undefined): string {
    if (!expect || expect === "none") return t("jobs.approval.expectNone");
    return t("jobs.approval.expectCron", { cron: describeCron(expect.cron, expect.tz), grace: expect.graceMinutes });
}

function triggerWords(trigger: Record<string, unknown> | null | undefined): string {
    if (!trigger) return t("jobs.approval.triggerNone");
    switch (trigger.kind) {
        case "cron":
            return typeof trigger.cron === "string"
                ? describeCron(trigger.cron, typeof trigger.tz === "string" ? trigger.tz : "UTC")
                : t("jobs.approval.triggerSchedule");
        case "every":
            return typeof trigger.seconds === "number"
                ? t("jobs.approval.triggerEvery", { n: Math.round(trigger.seconds / 60) })
                : t("jobs.approval.triggerInterval");
        case "once":
            return t("jobs.approval.triggerOnce");
        case "event":
            return t("jobs.approval.triggerEvent");
        default:
            return t("jobs.approval.triggerHand");
    }
}

function capsWords(policy: Record<string, unknown> | null | undefined): string {
    const caps = (policy?.caps ?? {}) as Record<string, unknown>;
    const parts: string[] = [];
    if (typeof caps.run_usd === "number") parts.push(t("jobs.approval.capRun", { usd: caps.run_usd }));
    if (typeof caps.day_usd === "number") parts.push(t("jobs.approval.capDay", { usd: caps.day_usd }));
    if (typeof caps.messages_per_day === "number") parts.push(t("jobs.approval.capMessages", { n: caps.messages_per_day }));
    return parts.length > 0 ? parts.join(", ") : t("jobs.approval.capDefaults");
}

/** A cloud template's facts: who owns it, the reporter whose key approving pins, what it is expected to do. */
const CloudFacts = (props: { manifest: JobTemplateManifest; reporter: ReporterNote | undefined }): JSX.Element => (
    <>
        <dt>{t("jobs.approval.owner")}</dt>
        <dd>{props.manifest.owner}</dd>
        <dt>{t("jobs.approval.reporter")}</dt>
        <dd>
            <Show when={props.reporter} fallback={<span class="warden-manager-dim">{t("jobs.approval.noneNamed")}</span>}>
                {(reporter) => (
                    <>
                        <Pill tone={reporterNote(reporter()).tone}>{reporter().name}</Pill>{" "}
                        <span>{reporterNote(reporter()).text}</span>
                    </>
                )}
            </Show>
        </dd>
        <dt>{t("jobs.approval.expects")}</dt>
        <dd>{expectWords(props.manifest.expect)}</dd>
        <dt>{t("jobs.approval.runLimit")}</dt>
        <dd>{t("jobs.approval.runLimitHelp", { n: props.manifest.maxDurationMinutes })}</dd>
        <Show when={props.manifest.runNow}>
            {(runNow) => (
                <>
                    <dt>{t("jobs.approval.runNow")}</dt>
                    <dd>
                        {runNow().link
                            ? t("jobs.approval.runNowOpens", { link: runNow().link })
                            : t("jobs.approval.runNowAsks", { owner: props.manifest.owner, ask: runNow().askOwner })}
                    </dd>
                </>
            )}
        </Show>
    </>
);

/** A job template's facts: when it triggers and what it does. */
const JobFacts = (props: { manifest: JobTemplateManifest }): JSX.Element => (
    <>
        <dt>{t("jobs.approval.triggers")}</dt>
        <dd>{triggerWords(props.manifest.trigger)}</dd>
        <dt>{t("jobs.approval.does")}</dt>
        <dd>
            {t("jobs.approval.doesJob", { kind: String((props.manifest.action as Record<string, unknown> | null)?.kind ?? "message") })}
            <Show when={props.manifest.readsUntrusted}>
                <span class="warden-manager-dim"> · {t("jobs.approval.untrusted")}</span>
            </Show>
        </dd>
    </>
);

/** A Drone template's facts: when it triggers, how big its graph is, and every call it makes outside AgentMux. */
const DroneFacts = (props: { manifest: JobTemplateManifest; drone: DroneSummary | undefined }): JSX.Element => (
    <>
        <dt>{t("jobs.approval.triggers")}</dt>
        <dd>{triggerWords(props.manifest.trigger)}</dd>
        <dt>{t("jobs.approval.does")}</dt>
        <dd>
            {t("jobs.approval.droneNodes", { count: props.drone?.nodes ?? 0 })}
            <Show when={(props.drone?.agents ?? 0) > 0}>, {t("jobs.approval.droneAgents", { n: props.drone?.agents ?? 0 })}</Show>
            <Show when={props.manifest.readsUntrusted}>
                <span class="warden-manager-dim"> · {t("jobs.approval.untrusted")}</span>
            </Show>
        </dd>
        <dt>{t("jobs.approval.callsOutside")}</dt>
        <dd>
            <Show
                when={(props.drone?.writes_outside.length ?? 0) > 0}
                fallback={<span class="warden-manager-dim">{t("jobs.approval.none")}</span>}
            >
                <For each={props.drone?.writes_outside}>{(call) => <Pill tone="warn">{call}</Pill>}</For>
            </Show>
        </dd>
        <dt>{t("templateApproval.usesCredential")}</dt>
        <dd>
            <Show
                when={(props.drone?.credentials?.length ?? 0) > 0}
                fallback={<span class="warden-manager-dim">{t("templateApproval.usesCredentialNo")}</span>}
            >
                {t("templateApproval.usesCredentialYes")}
                <For each={props.drone?.credentials}>
                    {(use) => <div style={{ "overflow-wrap": "anywhere" }}>{use}</div>}
                </For>
            </Show>
        </dd>
    </>
);

/** Every output the template declares; the ones that write outside AgentMux are marked. */
const OutputsFact = (props: { outputs: string[] }): JSX.Element => (
    <>
        <dt>{t("jobs.approval.outputs")}</dt>
        <dd>
            <Show when={props.outputs.length > 0} fallback={<span class="warden-manager-dim">{t("jobs.approval.none")}</span>}>
                <For each={props.outputs}>
                    {(o) => (
                        <Pill
                            tone={writesOutside(o) ? "warn" : "neutral"}
                            title={writesOutside(o) ? t("jobs.approval.writesOutside") : undefined}
                        >
                            {o}
                        </Pill>
                    )}
                </For>
            </Show>
        </dd>
    </>
);

/** Every secret the template declares. */
const SecretsFact = (props: { secrets: JobTemplateManifest["secrets"]; usesCredential?: boolean }): JSX.Element => (
    <>
        <dt>{t("jobs.approval.secrets")}</dt>
        <dd>
            <Show
                when={props.secrets.length > 0}
                fallback={
                    <span class="warden-manager-dim">
                        {props.usesCredential ? t("templateApproval.secretsNoneDeclared") : t("jobs.approval.none")}
                    </span>
                }
            >
                <For each={props.secrets}>{(s) => <Pill title={s.description ?? undefined}>{s.name}</Pill>}</For>
            </Show>
        </dd>
    </>
);

/** What the operator is asked to approve: the heading, the facts of this kind of template, and what approving does. */
const ApprovalFacts = (props: { info: JobTemplateInfo; manifest: JobTemplateManifest }): JSX.Element => {
    const note = () => publisherNote(props.info);
    return (
        <>
            <h3 class="warden-jobs-heading">
                {t("jobs.approval.heading", { name: props.manifest.name, version: props.manifest.version })}
            </h3>
            <p class="warden-manager-dim">{props.manifest.description}</p>
            <dl class="warden-jobs-facts">
                <dt>{t("jobs.approval.publisher")}</dt>
                <dd>
                    <Pill tone={note().tone}>{props.info.signature.publisher}</Pill> <span>{note().text}</span>
                </dd>
                <Show when={props.manifest.kind === "cloud"}>
                    <CloudFacts manifest={props.manifest} reporter={props.info.reporter} />
                </Show>
                <Show when={props.manifest.kind === "job"}>
                    <JobFacts manifest={props.manifest} />
                </Show>
                <Show when={props.manifest.kind === "drone"}>
                    <DroneFacts manifest={props.manifest} drone={props.info.drone} />
                </Show>
                <OutputsFact outputs={props.manifest.outputs} />
                <SecretsFact
                    secrets={props.manifest.secrets}
                    usesCredential={(props.info.drone?.credentials?.length ?? 0) > 0}
                />
                <Show when={props.manifest.kind !== "cloud"}>
                    <dt>{t("jobs.approval.defaultCaps")}</dt>
                    <dd>{capsWords(props.manifest.policy)}</dd>
                </Show>
            </dl>
            <p class="warden-manager-dim">
                <Show
                    when={props.manifest.kind === "cloud"}
                    fallback={t("jobs.approval.approvingJob")}
                >
                    {t("jobs.approval.approvingCloud")}
                </Show>
            </p>
        </>
    );
};

export function TemplateApproval(props: {
    info: JobTemplateInfo;
    busy: boolean;
    error: string | null;
    onApprove: () => void;
    onCancel: () => void;
}): JSX.Element {
    const m = () => props.info.manifest;
    const refused = () => {
        const reporter = props.info.reporter;
        return reporter ? reporterNote(reporter).block : false;
    };
    return (
        <section class="warden-jobs-approval" aria-label={t("jobs.approval.aria", { id: props.info.id })} data-operator-only="">
            <Show
                when={m()}
                fallback={<div class="warden-manager-error">{props.info.error ?? t("jobs.approval.unreadable")}</div>}
            >
                {(manifest) => <ApprovalFacts info={props.info} manifest={manifest()} />}
            </Show>
            <Show when={props.error}>
                <div class="warden-manager-error" role="alert">
                    {props.error}
                </div>
            </Show>
            <div class="warden-jobs-actions">
                <Button
                    tone="accent"
                    icon="check"
                    busy={props.busy}
                    disabled={!m() || refused()}
                    onClick={() => props.onApprove()}
                >
                    {t("jobs.approval.approve")}
                </Button>
                <Button tone="quiet" disabled={props.busy} onClick={() => props.onCancel()}>
                    {t("jobs.approval.cancel")}
                </Button>
            </div>
        </section>
    );
}
