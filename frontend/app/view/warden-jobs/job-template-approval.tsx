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
import type { CloudExpect } from "@/types/rpc/CloudExpect";
import type { DroneSummary } from "@/types/rpc/DroneSummary";
import type { JobTemplateInfo } from "@/types/rpc/JobTemplateInfo";
import type { JobTemplateManifest } from "@/types/rpc/JobTemplateManifest";
import type { ReporterNote } from "@/types/rpc/ReporterNote";

import { describeCron } from "./jobs-format";

import "./job-editor.scss";

/** Outputs that write outside AgentMux or to someone else (srv's `OUTSIDE_OUTPUTS`). */
const OUTSIDE = new Set(["pull_request", "comment", "message:others", "deploy"]);

const writesOutside = (output: string): boolean => OUTSIDE.has(output) || OUTSIDE.has(output.split(":")[0]);

/** The publisher's key, in words and in a tone. */
export function publisherNote(info: JobTemplateInfo): { text: string; tone: PillTone } {
    const sig = info.signature;
    if (info.source === "builtin") return { text: "Published by AgentMux and part of this app.", tone: "ok" };
    switch (sig.state) {
        case "signed":
            return { text: `Signed with the key pinned for ${sig.publisher} (${sig.fingerprint}).`, tone: "ok" };
        case "signed_new":
            return { text: `${sig.publisher} is new to you. Approving pins its key ${sig.fingerprint}.`, tone: "warn" };
        case "key_changed":
            return {
                text: `The key differs from the one pinned for ${sig.publisher}: pinned ${sig.pinned ?? "none"}, this one ${sig.fingerprint ?? "unsigned"}.`,
                tone: "error",
            };
        default:
            return { text: `Unsigned. Nothing vouches that ${sig.publisher} wrote it.`, tone: "warn" };
    }
}

/** What approving does to the reporter's pin, in words and in a tone; `block` when approving would be refused. */
export function reporterNote(note: ReporterNote): { text: string; tone: PillTone; block: boolean } {
    switch (note.state) {
        case "new":
            return {
                text: `New to you. Approving pins its key ${note.fingerprint} and accepts the signed run reports it sends for this job at once.`,
                tone: "warn",
                block: false,
            };
        case "pinned":
            return { text: `Already pinned (${note.fingerprint}).`, tone: "ok", block: false };
        case "name_taken":
            return {
                text: `The name ${note.name} is pinned to a different key than this one (${note.fingerprint}). Forget that reporter first; approving is refused until then.`,
                tone: "error",
                block: true,
            };
        default:
            return {
                text: `This key (${note.fingerprint}) is already pinned under another name. Approving is refused.`,
                tone: "error",
                block: true,
            };
    }
}

function expectWords(expect: CloudExpect | null | undefined): string {
    if (!expect || expect === "none") return "no schedule: events start it";
    return `${describeCron(expect.cron, expect.tz)}, a report is missed ${expect.graceMinutes} min after its slot`;
}

function triggerWords(trigger: Record<string, unknown> | null | undefined): string {
    if (!trigger) return "none (run by hand)";
    switch (trigger.kind) {
        case "cron":
            return typeof trigger.cron === "string"
                ? describeCron(trigger.cron, typeof trigger.tz === "string" ? trigger.tz : "UTC")
                : "on a schedule";
        case "every":
            return typeof trigger.seconds === "number"
                ? `every ${Math.round(trigger.seconds / 60)} min`
                : "on an interval";
        case "once":
            return "once";
        case "event":
            return "on an event";
        default:
            return "by hand";
    }
}

function capsWords(policy: Record<string, unknown> | null | undefined): string {
    const caps = (policy?.caps ?? {}) as Record<string, unknown>;
    const parts: string[] = [];
    if (typeof caps.run_usd === "number") parts.push(`$${caps.run_usd} per run`);
    if (typeof caps.day_usd === "number") parts.push(`$${caps.day_usd} per day`);
    if (typeof caps.messages_per_day === "number") parts.push(`${caps.messages_per_day} messages per day`);
    return parts.length > 0 ? parts.join(", ") : "the defaults";
}

/** A cloud template's facts: who owns it, the reporter whose key approving pins, what it is expected to do. */
const CloudFacts = (props: { manifest: JobTemplateManifest; reporter: ReporterNote | undefined }): JSX.Element => (
    <>
        <dt>Owner</dt>
        <dd>{props.manifest.owner}</dd>
        <dt>Reporter</dt>
        <dd>
            <Show when={props.reporter} fallback={<span class="warden-manager-dim">none named</span>}>
                {(reporter) => (
                    <>
                        <Pill tone={reporterNote(reporter()).tone}>{reporter().name}</Pill>{" "}
                        <span>{reporterNote(reporter()).text}</span>
                    </>
                )}
            </Show>
        </dd>
        <dt>Expects</dt>
        <dd>{expectWords(props.manifest.expect)}</dd>
        <dt>Run limit</dt>
        <dd>{props.manifest.maxDurationMinutes} min before a run counts as lost</dd>
        <Show when={props.manifest.runNow}>
            {(runNow) => (
                <>
                    <dt>Run now</dt>
                    <dd>
                        {runNow().link
                            ? `opens ${runNow().link}`
                            : `asks ${props.manifest.owner}: ${runNow().askOwner}`}
                    </dd>
                </>
            )}
        </Show>
    </>
);

/** A job template's facts: when it triggers and what it does. */
const JobFacts = (props: { manifest: JobTemplateManifest }): JSX.Element => (
    <>
        <dt>Triggers</dt>
        <dd>{triggerWords(props.manifest.trigger)}</dd>
        <dt>Does</dt>
        <dd>
            {String((props.manifest.action as Record<string, unknown> | null)?.kind ?? "message")} an agent with the
            template's brief
            <Show when={props.manifest.readsUntrusted}>
                <span class="warden-manager-dim"> · reads text nobody vouches for</span>
            </Show>
        </dd>
    </>
);

/** A Drone template's facts: when it triggers, how big its graph is, and every call it makes outside AgentMux. */
const DroneFacts = (props: { manifest: JobTemplateManifest; drone: DroneSummary | undefined }): JSX.Element => (
    <>
        <dt>Triggers</dt>
        <dd>{triggerWords(props.manifest.trigger)}</dd>
        <dt>Does</dt>
        <dd>
            runs a drone of {props.drone?.nodes ?? 0} {props.drone?.nodes === 1 ? "node" : "nodes"}
            <Show when={(props.drone?.agents ?? 0) > 0}>, {props.drone?.agents} of them start an agent</Show>
            <Show when={props.manifest.readsUntrusted}>
                <span class="warden-manager-dim"> · reads text nobody vouches for</span>
            </Show>
        </dd>
        <dt>Calls outside</dt>
        <dd>
            <Show
                when={(props.drone?.writes_outside.length ?? 0) > 0}
                fallback={<span class="warden-manager-dim">none</span>}
            >
                <For each={props.drone?.writes_outside}>{(call) => <Pill tone="warn">{call}</Pill>}</For>
            </Show>
        </dd>
    </>
);

/** Every output the template declares; the ones that write outside AgentMux are marked. */
const OutputsFact = (props: { outputs: string[] }): JSX.Element => (
    <>
        <dt>Outputs</dt>
        <dd>
            <Show when={props.outputs.length > 0} fallback={<span class="warden-manager-dim">none</span>}>
                <For each={props.outputs}>
                    {(o) => (
                        <Pill
                            tone={writesOutside(o) ? "warn" : "neutral"}
                            title={writesOutside(o) ? "Writes outside AgentMux" : undefined}
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
const SecretsFact = (props: { secrets: JobTemplateManifest["secrets"] }): JSX.Element => (
    <>
        <dt>Secrets</dt>
        <dd>
            <Show when={props.secrets.length > 0} fallback={<span class="warden-manager-dim">none</span>}>
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
                Approve {props.manifest.name} {props.manifest.version}?
            </h3>
            <p class="warden-manager-dim">{props.manifest.description}</p>
            <dl class="warden-jobs-facts">
                <dt>Publisher</dt>
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
                <SecretsFact secrets={props.manifest.secrets} />
                <Show when={props.manifest.kind !== "cloud"}>
                    <dt>Default caps</dt>
                    <dd>{capsWords(props.manifest.policy)}</dd>
                </Show>
            </dl>
            <p class="warden-manager-dim">
                <Show
                    when={props.manifest.kind === "cloud"}
                    fallback="Approving runs nothing. It lets you make jobs from exactly this version; a changed version asks again."
                >
                    Approving starts nothing: AgentMux holds no cloud credentials. It pins the reporter's key and lets
                    this job's signed run reports into the Jobs ledger; a changed version asks again.
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
        <section class="warden-jobs-approval" aria-label={`Approve ${props.info.id}`}>
            <Show
                when={m()}
                fallback={<div class="warden-manager-error">{props.info.error ?? "This template can't be read."}</div>}
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
                    Approve
                </Button>
                <Button tone="quiet" disabled={props.busy} onClick={() => props.onCancel()}>
                    Cancel
                </Button>
            </div>
        </section>
    );
}
