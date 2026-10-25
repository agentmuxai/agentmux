// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import {
    blankDraft,
    buildDefinition,
    draftFromDefinition,
    draftFromTemplate,
    paramForm,
    paramKind,
    paramNames,
    paramsFromForm,
    toLocalInput,
} from "./job-draft";
import { mkTemplate } from "./jobs-fixtures";

const manifest = () => mkTemplate().manifest!;

describe("a blank job's definition", () => {
    it("is the message job the form describes, with the policy fields as numbers", () => {
        const d = blankDraft("Europe/Berlin");
        Object.assign(d, {
            name: " Nightly ",
            owner: "uid-o",
            to: "uid-b",
            brief: "Check in.",
            timeoutMinutes: "30",
            runCap: "12.5",
            messagesPerDay: "6",
        });
        const built = buildDefinition(d, null);
        expect(built.errors).toEqual([]);
        expect(built.definition).toEqual({
            jobVersion: 1,
            name: "Nightly",
            owner: "uid-o",
            enabled: true,
            params: {},
            trigger: { kind: "cron", cron: "0 9 * * 1", tz: "Europe/Berlin" },
            action: { kind: "message", to: "uid-b", brief: "Check in." },
            policy: {
                overlap: "skip",
                catch_up: "skip",
                retries: 0,
                timeout_minutes: 30,
                caps: { run_usd: 12.5, messages_per_day: 6 },
            },
        });
    });

    it("builds each trigger and action kind", () => {
        const d = blankDraft("UTC");
        Object.assign(d, {
            name: "n",
            triggerKind: "every",
            everyMinutes: "90",
            actionKind: "task",
            title: "Triage",
            payload: "Look.",
            taskTarget: "uid-b",
        });
        let built = buildDefinition(d, null);
        expect(built.definition.trigger).toEqual({ kind: "every", seconds: 5400 });
        expect(built.definition.action).toEqual({ kind: "task", title: "Triage", target: "uid-b", payload: "Look." });
        Object.assign(d, {
            triggerKind: "manual",
            actionKind: "spawn",
            leader: "uid-l",
            brief: "Do it.",
            model: "m1",
            keepWorker: true,
        });
        built = buildDefinition(d, null);
        expect(built.definition.trigger).toEqual({ kind: "manual" });
        expect(built.definition.action).toEqual({
            kind: "spawn",
            leader: "uid-l",
            brief: "Do it.",
            model: "m1",
            keep_worker: true,
        });
        Object.assign(d, { triggerKind: "once", onceAt: "2030-01-02T03:04" });
        expect((buildDefinition(d, null).definition.trigger as { at: number }).at).toBe(
            new Date("2030-01-02T03:04").getTime()
        );
        expect(toLocalInput(new Date("2030-01-02T03:04").getTime())).toBe("2030-01-02T03:04");
    });

    it("says what is missing instead of building it", () => {
        const d = blankDraft("UTC");
        Object.assign(d, { cron: " ", timeoutMinutes: "1.5", runCap: "-2" });
        const { errors } = buildDefinition(d, null);
        expect(errors).toEqual(
            expect.arrayContaining([
                "Give the job a name.",
                "Give the cron expression: minute hour day month weekday.",
                "Choose the agent to message.",
                "Write the brief the agent gets.",
                "Timeout (minutes): a whole number.",
                "Cap per run: dollars, 0 or more.",
            ])
        );
        const once = { ...blankDraft("UTC"), name: "n", to: "a", brief: "b", triggerKind: "once" as const };
        expect(buildDefinition(once, null).errors).toContain("Once: pick the date and time.");
        const every = {
            ...blankDraft("UTC"),
            name: "n",
            to: "a",
            brief: "b",
            triggerKind: "every" as const,
            everyMinutes: "0",
        };
        expect(buildDefinition(every, null).errors).toContain("Every: a number of minutes above 0.");
    });
});

describe("a template's parameters", () => {
    it("are ordered with the required ones first and typed from the schema", () => {
        const m = manifest();
        expect(paramNames(m)).toEqual(["repo", "focus", "max_items", "paths", "token"]);
        expect(Object.values(m.params.properties).map(paramKind)).toEqual([
            "string",
            "enum",
            "list",
            "number",
            "credential",
        ]);
        expect(paramForm(m)).toEqual({ repo: "", focus: "security", paths: "", max_items: "10", token: "" });
        expect(paramForm(m, { repo: "acme/site", paths: ["docs", "src"] })).toMatchObject({
            repo: "acme/site",
            paths: "docs\nsrc",
        });
    });

    it("go back as typed values, and a required one left empty is an error", () => {
        const m = manifest();
        const form = { ...paramForm(m), repo: " acme/site ", paths: "docs\n\n src ", max_items: "7", token: "v-1" };
        expect(paramsFromForm(m, form)).toEqual({
            params: { repo: "acme/site", focus: "security", paths: ["docs", "src"], max_items: 7, token: "v-1" },
            errors: {},
        });
        const bad = paramsFromForm(m, { ...paramForm(m), repo: "", max_items: "lots" });
        expect(bad.errors).toEqual({ repo: "Required.", max_items: "A number." });
    });
});

describe("a draft from a template", () => {
    it("takes the template's trigger, action and policy, and pins it", () => {
        const d = draftFromTemplate(mkTemplate(), "UTC");
        expect(d).toMatchObject({
            name: "Weekly report",
            template: { id: "agentmux.weekly-report", version: "1.0.0", hash: "hash-1" },
            triggerKind: "cron",
            cron: "0 6 * * 1",
            actionKind: "message",
            catchUp: "latest",
            timeoutMinutes: "60",
            runCap: "3",
        });
        d.to = "uid-b";
        d.params.repo = "acme/site";
        const built = buildDefinition(d, manifest());
        expect(built.errors).toEqual([]);
        expect(built.definition).toMatchObject({
            template: { id: "agentmux.weekly-report", version: "1.0.0", hash: "hash-1" },
            params: { repo: "acme/site", focus: "security", max_items: 10 },
            action: { kind: "message", to: "uid-b" },
        });
        expect(built.definition.outputs).toBeUndefined();
        expect((built.definition.action as Record<string, unknown>).brief).toBeUndefined();
    });

    it("needs the template's required settings", () => {
        const d = draftFromTemplate(mkTemplate(), "UTC");
        d.to = "uid-b";
        const built = buildDefinition(d, manifest());
        expect(built.paramErrors).toEqual({ repo: "Required." });
        expect(built.errors).toContain("Some of the template's settings need a value.");
    });
});

describe("a draft from a stored job", () => {
    const stored = {
        jobVersion: 1,
        id: "j1",
        name: "Docs drift",
        owner: "uid-o",
        template: { id: "agentmux.weekly-report", version: "1.0.0", hash: "hash-0" },
        params: { repo: "acme/site" },
        trigger: { kind: "cron", cron: "0 5 * * 2", tz: "America/New_York" },
        action: { kind: "message", to: "uid-b", brief: "Rendered." },
        policy: {
            overlap: "buffer_one",
            catch_up: "latest",
            retries: 2,
            max_turns: 50,
            approval: "none",
            caps: { day_usd: 40 },
        },
        outputs: ["report_file"],
        enabled: false,
    };

    it("edits in place and sends back what the form has no field for", () => {
        const d = draftFromDefinition(stored, manifest(), "edit");
        expect(d).toMatchObject({
            id: "j1",
            name: "Docs drift",
            enabled: false,
            overlap: "buffer_one",
            dayCap: "40",
            tz: "America/New_York",
            template: { hash: "hash-0" },
        });
        expect(d.params.repo).toBe("acme/site");
        const built = buildDefinition(d, manifest());
        expect(built.definition.policy).toMatchObject({
            max_turns: 50,
            approval: "none",
            retries: 2,
            caps: { day_usd: 40 },
        });
        expect(built.definition.enabled).toBe(false);
    });

    it("clones as a new job, with a copy's name and its template at the installed version", () => {
        const d = draftFromDefinition(stored, manifest(), "clone");
        expect(d.id).toBeNull();
        expect(d.name).toBe("Docs drift (copy)");
        expect(d.template).toEqual({ id: "agentmux.weekly-report", version: "", hash: "" });
    });

    it("keeps a job without a template as it is, outputs and secrets included", () => {
        const plain = {
            ...stored,
            template: undefined,
            outputs: ["kanban_task"],
            secrets: [{ name: "T", vault: "v" }],
            params: { a: 1 },
        };
        const built = buildDefinition(draftFromDefinition(plain, null, "edit"), null);
        expect(built.definition).toMatchObject({
            outputs: ["kanban_task"],
            secrets: [{ name: "T", vault: "v" }],
            params: { a: 1 },
        });
        expect(built.definition.template).toBeUndefined();
    });

    it("sends a once job's stored time back as it was unless the date field is changed", () => {
        const at = new Date(2030, 0, 2, 3, 4, 27, 500).getTime();
        const stored = {
            name: "n",
            owner: "a",
            brief: "b",
            action: { kind: "message", to: "a" },
            trigger: { kind: "once", at },
        };
        const d = draftFromDefinition(stored, null, "edit");
        d.name = "renamed";
        expect((buildDefinition(d, null).definition.trigger as { at: number }).at).toBe(at);
        d.onceAt = "2030-02-03T04:05";
        expect((buildDefinition(d, null).definition.trigger as { at: number }).at).toBe(
            new Date("2030-02-03T04:05").getTime()
        );
    });
});
