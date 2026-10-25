// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The editor's draft of a job: what the form holds, how it is made from a blank job, a template or a
// stored job (Edit, Clone), and how it becomes the definition `jobs.save` takes
// (schema/agentmux-job.v1.schema.json, docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §5.1, §5.4).
// srv checks the definition again; this only keeps a form from sending what is plainly incomplete.

import type { JobTemplateInfo } from "@/types/rpc/JobTemplateInfo";
import type { JobTemplateManifest } from "@/types/rpc/JobTemplateManifest";
import type { JobTemplateParam } from "@/types/rpc/JobTemplateParam";

export type TriggerKind = "cron" | "every" | "once" | "manual";
export type ActionKind = "message" | "task" | "spawn";
export type Json = Record<string, unknown>;

/** A parameter's form value: text for most kinds, on or off for a boolean. */
export type ParamValue = string | boolean;
export type ParamKind = "string" | "number" | "boolean" | "enum" | "list" | "credential";

/** The template a job is made from. `version` and `hash` are empty for "the installed one". */
export interface TemplatePin {
    id: string;
    version: string;
    hash: string;
}

export interface JobDraft {
    /** The job being edited; null for a new job or a clone. */
    id: string | null;
    name: string;
    /** The owning agent's UID. */
    owner: string;
    enabled: boolean;
    template: TemplatePin | null;
    params: Record<string, ParamValue>;
    triggerKind: TriggerKind;
    cron: string;
    tz: string;
    everyMinutes: string;
    onceAt: string;
    /** The stored time a `once` job was loaded with, and how the date field showed it: the field has
     *  minute precision, so an untouched field sends the stored time back as it was. */
    onceStored: { at: number; shown: string } | null;
    actionKind: ActionKind;
    /** A message's target. */
    to: string;
    brief: string;
    title: string;
    payload: string;
    taskTarget: string;
    leader: string;
    model: string;
    account: string;
    keepWorker: boolean;
    overlap: string;
    catchUp: string;
    timeoutMinutes: string;
    retries: string;
    group: string;
    runCap: string;
    dayCap: string;
    messagesPerDay: string;
    /** Carried through as stored: the editor has no field for them. */
    keep: Kept;
}

export interface Kept {
    /** Policy settings the form doesn't show: `max_turns`, `approval`, `results`. */
    policy: Json;
    /** A job keeps its parameters as stored when its template can't be read; one without a template
     *  keeps its declared outputs and secrets too. */
    outputs: unknown[] | null;
    secrets: unknown[] | null;
    params: Json;
}

const nothingKept = (): Kept => ({ policy: {}, outputs: null, secrets: null, params: {} });

export const OVERLAPS = ["skip", "buffer_one", "cancel_other"];
export const CATCH_UPS = ["skip", "latest"];

/** The viewer's own time zone, the default for a new cron trigger. */
export function localZone(): string {
    try {
        return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
    } catch {
        return "UTC";
    }
}

export function blankDraft(tz: string = localZone()): JobDraft {
    return {
        id: null,
        name: "",
        owner: "",
        enabled: true,
        template: null,
        params: {},
        triggerKind: "cron",
        cron: "0 9 * * 1",
        tz,
        everyMinutes: "60",
        onceAt: "",
        onceStored: null,
        actionKind: "message",
        to: "",
        brief: "",
        title: "",
        payload: "",
        taskTarget: "",
        leader: "",
        model: "",
        account: "",
        keepWorker: false,
        overlap: "skip",
        catchUp: "skip",
        timeoutMinutes: "",
        retries: "0",
        group: "",
        runCap: "",
        dayCap: "",
        messagesPerDay: "",
        keep: nothingKept(),
    };
}

// ── Parameters ──────────────────────────────────────────────────────────────

export function paramKind(p: JobTemplateParam): ParamKind {
    if (p.enum && p.enum.length > 0) return "enum";
    if (p.format === "credential") return "credential";
    if (p.type === "number") return "number";
    if (p.type === "boolean") return "boolean";
    if (p.type === "array") return "list";
    return "string";
}

/** The parameters in the order the form shows them: required ones first, then by name. */
export function paramNames(m: JobTemplateManifest): string[] {
    const required = new Set(m.params.required);
    return Object.keys(m.params.properties).sort(
        (a, b) => Number(required.has(b)) - Number(required.has(a)) || a.localeCompare(b)
    );
}

function formValue(p: JobTemplateParam, value: unknown): ParamValue {
    switch (paramKind(p)) {
        case "boolean":
            return value === true;
        case "list":
            return Array.isArray(value) ? value.filter((v) => typeof v === "string").join("\n") : "";
        default:
            return value == null ? "" : String(value);
    }
}

/** The form values for `m`'s parameters: `given` where it has one, else the template's default. */
export function paramForm(m: JobTemplateManifest, given: Json = {}): Record<string, ParamValue> {
    const out: Record<string, ParamValue> = {};
    for (const [name, p] of Object.entries(m.params.properties))
        out[name] = formValue(p, name in given ? given[name] : p.default);
    return out;
}

/** The parameters as the definition has them, and a message for each one that can't be sent. `stored`
 *  is the job's own parameters: a switch left off is sent only when the job, the template's default or
 *  the template's `required` list gives it a value, so editing another field doesn't add a key. */
export function paramsFromForm(
    m: JobTemplateManifest,
    form: Record<string, ParamValue>,
    stored: Json = {}
): { params: Json; errors: Record<string, string> } {
    const params: Json = {};
    const errors: Record<string, string> = {};
    const required = new Set(m.params.required);
    for (const [name, p] of Object.entries(m.params.properties)) {
        const raw = form[name];
        const kind = paramKind(p);
        if (kind === "boolean") {
            if (raw === true || name in stored || p.default !== undefined || required.has(name))
                params[name] = raw === true;
            continue;
        }
        const text = typeof raw === "string" ? raw.trim() : "";
        if (kind === "list") {
            const items = text
                .split(/\r?\n/)
                .map((l) => l.trim())
                .filter(Boolean);
            if (items.length > 0) params[name] = items;
            else if (required.has(name)) errors[name] = "Required.";
            continue;
        }
        if (text === "") {
            if (required.has(name)) errors[name] = "Required.";
            continue;
        }
        if (kind === "number") {
            const n = Number(text);
            if (Number.isFinite(n)) params[name] = n;
            else errors[name] = "A number.";
        } else {
            params[name] = text;
        }
    }
    return { params, errors };
}

// ── Drafts from a template and from a stored job ───────────────────────────

const str = (v: unknown): string => (typeof v === "string" ? v : "");
const num = (v: unknown): string => (typeof v === "number" && Number.isFinite(v) ? String(v) : "");
const obj = (v: unknown): Json => (v && typeof v === "object" && !Array.isArray(v) ? (v as Json) : {});

function applyTrigger(d: JobDraft, trigger: Json): void {
    const kind = str(trigger.kind);
    if (kind === "cron" || kind === "every" || kind === "once" || kind === "manual") d.triggerKind = kind;
    if (kind === "cron") {
        d.cron = str(trigger.cron) || d.cron;
        d.tz = str(trigger.tz) || d.tz;
    }
    if (kind === "every" && typeof trigger.seconds === "number")
        d.everyMinutes = String(Math.round((trigger.seconds / 60) * 100) / 100);
    if (kind === "once" && typeof trigger.at === "number") {
        d.onceAt = toLocalInput(trigger.at);
        d.onceStored = { at: trigger.at, shown: d.onceAt };
    }
}

function applyPolicy(d: JobDraft, policy: Json): void {
    d.overlap = str(policy.overlap) || d.overlap;
    d.catchUp = str(policy.catch_up) || d.catchUp;
    d.timeoutMinutes = num(policy.timeout_minutes) || d.timeoutMinutes;
    d.retries = num(policy.retries) || d.retries;
    d.group = str(policy.group) || d.group;
    const caps = obj(policy.caps);
    d.runCap = num(caps.run_usd) || d.runCap;
    d.dayCap = num(caps.day_usd) || d.dayCap;
    d.messagesPerDay = num(caps.messages_per_day) || d.messagesPerDay;
}

/** A new job from template `info`, with the template's defaults. */
export function draftFromTemplate(info: JobTemplateInfo, tz: string = localZone()): JobDraft {
    const d = blankDraft(tz);
    const m = info.manifest;
    if (!m) return d;
    d.name = m.name;
    d.template = { id: m.id, version: m.version, hash: info.hash };
    d.params = paramForm(m);
    applyTrigger(d, obj(m.trigger));
    applyPolicy(d, obj(m.policy));
    const action = str(obj(m.action).kind);
    if (action === "message" || action === "task" || action === "spawn") d.actionKind = action;
    return d;
}

/** A job's stored definition as a draft. A clone is a new job: no id, "(copy)" in its name, and its
 *  template at the installed version (it can't be made from an old one). */
export function draftFromDefinition(def: Json, manifest: JobTemplateManifest | null, mode: "edit" | "clone"): JobDraft {
    const d = blankDraft();
    d.id = mode === "edit" ? str(def.id) || null : null;
    d.name = mode === "clone" ? `${str(def.name)} (copy)` : str(def.name);
    d.owner = str(def.owner);
    d.enabled = def.enabled !== false;
    const t = obj(def.template);
    if (str(t.id))
        d.template =
            mode === "edit"
                ? { id: str(t.id), version: str(t.version), hash: str(t.hash) }
                : { id: str(t.id), version: "", hash: "" };
    d.params = manifest ? paramForm(manifest, obj(def.params)) : {};
    applyTrigger(d, obj(def.trigger));
    const action = obj(def.action);
    const kind = str(action.kind);
    if (kind === "message" || kind === "task" || kind === "spawn") d.actionKind = kind;
    d.to = str(action.to);
    d.brief = str(action.brief);
    d.title = str(action.title);
    d.payload = str(action.payload);
    d.taskTarget = str(action.target);
    d.leader = str(action.leader);
    d.model = str(action.model);
    d.account = str(action.account);
    d.keepWorker = action.keep_worker === true;
    const policy = obj(def.policy);
    applyPolicy(d, policy);
    // What the form has no field for goes back as it was.
    d.keep = nothingKept();
    for (const key of ["max_turns", "approval", "results"]) if (key in policy) d.keep.policy[key] = policy[key];
    d.keep.params = obj(def.params);
    if (!d.template) {
        d.keep.outputs = Array.isArray(def.outputs) ? def.outputs : null;
        d.keep.secrets = Array.isArray(def.secrets) ? def.secrets : null;
    }
    return d;
}

// ── A datetime-local input and ms epoch ─────────────────────────────────────

const pad = (n: number): string => String(n).padStart(2, "0");

/** `ms` as a `datetime-local` value, in the viewer's zone. */
export function toLocalInput(ms: number): string {
    const t = new Date(ms);
    return `${t.getFullYear()}-${pad(t.getMonth() + 1)}-${pad(t.getDate())}T${pad(t.getHours())}:${pad(t.getMinutes())}`;
}

// ── The definition ──────────────────────────────────────────────────────────

export interface BuiltDefinition {
    definition: Json;
    /** What stops it being saved, in words; empty when it can be sent. */
    errors: string[];
    /** Per parameter, for the form. */
    paramErrors: Record<string, string>;
}

const wholeNumber = (text: string): number | null => {
    const n = Number(text);
    return text.trim() !== "" && Number.isInteger(n) && n >= 0 ? n : null;
};

function buildTrigger(d: JobDraft, errors: string[]): Json {
    switch (d.triggerKind) {
        case "cron":
            if (!d.cron.trim()) errors.push("Give the cron expression: minute hour day month weekday.");
            return { kind: "cron", cron: d.cron.trim(), tz: d.tz.trim() || "UTC" };
        case "every": {
            const minutes = Number(d.everyMinutes);
            if (!(minutes > 0)) errors.push("Every: a number of minutes above 0.");
            return { kind: "every", seconds: Math.round(minutes * 60) };
        }
        case "once": {
            const untouched = d.onceStored != null && d.onceStored.shown === d.onceAt;
            const at = untouched ? d.onceStored!.at : d.onceAt ? new Date(d.onceAt).getTime() : NaN;
            if (!Number.isFinite(at)) errors.push("Once: pick the date and time.");
            return { kind: "once", at };
        }
        default:
            return { kind: "manual" };
    }
}

function buildAction(d: JobDraft, errors: string[]): Json {
    const templated = d.template != null;
    const need = (value: string, what: string) => {
        if (!value.trim()) errors.push(what);
    };
    switch (d.actionKind) {
        case "message":
            need(d.to, "Choose the agent to message.");
            if (!templated) need(d.brief, "Write the brief the agent gets.");
            // A templated job's brief is the template's; the stored one goes back so an edit that
            // leaves it alone isn't rendered again.
            return d.brief ? { kind: "message", to: d.to, brief: d.brief } : { kind: "message", to: d.to };
        case "task": {
            need(d.title, "Give the task a title.");
            const action: Json = { kind: "task", title: d.title.trim() };
            if (d.taskTarget) action.target = d.taskTarget;
            if (d.payload) action.payload = d.payload;
            return action;
        }
        default: {
            need(d.leader, "Choose the leader of the worker.");
            if (!templated) need(d.brief, "Write the brief the worker gets.");
            const action: Json = { kind: "spawn", leader: d.leader };
            if (d.brief) action.brief = d.brief;
            if (d.model.trim()) action.model = d.model.trim();
            if (d.account.trim()) action.account = d.account.trim();
            if (d.keepWorker) action.keep_worker = true;
            return action;
        }
    }
}

function buildPolicy(d: JobDraft, errors: string[]): Json {
    const policy: Json = { overlap: d.overlap, catch_up: d.catchUp, ...d.keep.policy };
    const whole = (text: string, what: string, apply: (n: number) => void) => {
        if (text.trim() === "") return;
        const n = wholeNumber(text);
        if (n == null) errors.push(`${what}: a whole number.`);
        else apply(n);
    };
    whole(d.timeoutMinutes, "Timeout (minutes)", (n) => (policy.timeout_minutes = n));
    whole(d.retries, "Retries", (n) => (policy.retries = n));
    if (d.group.trim()) policy.group = d.group.trim();
    const caps: Json = {};
    for (const [text, key, what] of [
        [d.runCap, "run_usd", "Cap per run"],
        [d.dayCap, "day_usd", "Cap per day"],
    ] as const) {
        if (text.trim() === "") continue;
        const n = Number(text);
        if (Number.isFinite(n) && n >= 0) caps[key] = n;
        else errors.push(`${what}: dollars, 0 or more.`);
    }
    whole(d.messagesPerDay, "Messages per day", (n) => (caps.messages_per_day = n));
    if (Object.keys(caps).length > 0) policy.caps = caps;
    return policy;
}

/** The definition `jobs.save` takes for `draft`, with what stops it being sent. `manifest` is the
 *  draft's template. */
export function buildDefinition(d: JobDraft, manifest: JobTemplateManifest | null): BuiltDefinition {
    const errors: string[] = [];
    if (!d.name.trim()) errors.push("Give the job a name.");
    let params: Json = {};
    let paramErrors: Record<string, string> = {};
    if (d.template && manifest) {
        ({ params, errors: paramErrors } = paramsFromForm(manifest, d.params, d.keep.params));
        if (Object.keys(paramErrors).length > 0) errors.push("Some of the template's settings need a value.");
    } else {
        params = d.keep.params;
    }
    const definition: Json = {
        jobVersion: 1,
        name: d.name.trim(),
        owner: d.owner,
        enabled: d.enabled,
        params,
        trigger: buildTrigger(d, errors),
        action: buildAction(d, errors),
        policy: buildPolicy(d, errors),
    };
    if (d.template) definition.template = { id: d.template.id, version: d.template.version, hash: d.template.hash };
    else {
        if (d.keep.outputs) definition.outputs = d.keep.outputs;
        if (d.keep.secrets) definition.secrets = d.keep.secrets;
    }
    return { definition, errors, paramErrors };
}
