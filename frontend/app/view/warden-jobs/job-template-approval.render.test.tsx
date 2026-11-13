// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// TemplateApproval, row by row: the heading and description, what each publisher key and each reporter
// pin says (and in which tone), the facts of a job template (trigger, action, caps) and of a cloud one
// (owner, reporter, schedule, run limit, run now), the output and secret pills, the footnote, the error,
// and when Approve and Cancel are available. job-template-approval.test.tsx covers a cloud template's
// prompt as a whole; this file covers the words and the branches.

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { JobTemplateInfo } from "@/types/rpc/JobTemplateInfo";
import type { JobTemplateManifest } from "@/types/rpc/JobTemplateManifest";
import type { ReporterNote } from "@/types/rpc/ReporterNote";

import { publisherNote, reporterNote, TemplateApproval } from "./job-template-approval";
import { mkTemplate } from "./jobs-fixtures";

afterEach(cleanup);

/** A job template (the fixture's weekly report) with some manifest fields and some info fields changed. */
function job(manifest: Partial<JobTemplateManifest> = {}, info: Partial<JobTemplateInfo> = {}): JobTemplateInfo {
    const base = mkTemplate({ state: "needs_approval", ...info });
    return { ...base, manifest: { ...base.manifest!, ...manifest } };
}

/** A cloud template: no trigger, action or caps, but an owner, a reporter, a schedule and a run limit. */
function cloud(manifest: Partial<JobTemplateManifest> = {}, info: Partial<JobTemplateInfo> = {}): JobTemplateInfo {
    return job(
        {
            id: "acme.nightly",
            name: "Nightly build",
            kind: "cloud",
            action: null,
            trigger: null,
            policy: null,
            outputs: [],
            secrets: [],
            owner: "ops",
            reporter: { name: "acme-ci", key: "AAAA" },
            expect: { cron: "0 2 * * *", tz: "UTC", graceMinutes: 30 },
            maxDurationMinutes: 120,
            runNow: { askOwner: "Run the nightly workflow" },
            ...manifest,
        },
        { id: "acme.nightly", source: "installed", ...info }
    );
}

/** A template srv could not read: no manifest, and its reason. */
const unreadable = (error: string | null): JobTemplateInfo => ({
    ...mkTemplate({ state: "invalid" }),
    manifest: null,
    error,
});

const pin = (state: ReporterNote["state"], over: Partial<ReporterNote> = {}): ReporterNote => ({
    name: "acme-ci",
    fingerprint: "K7Q2-MZ4D-PX3A-9TWE",
    state,
    ...over,
});

interface Mounted {
    onApprove: ReturnType<typeof vi.fn>;
    onCancel: ReturnType<typeof vi.fn>;
}

function show(info: JobTemplateInfo, props: { busy?: boolean; error?: string | null } = {}): Mounted {
    const onApprove = vi.fn();
    const onCancel = vi.fn();
    render(() => (
        <TemplateApproval
            info={info}
            busy={props.busy ?? false}
            error={props.error ?? null}
            onApprove={onApprove}
            onCancel={onCancel}
        />
    ));
    return { onApprove, onCancel };
}

/** The facts list as label → value element. */
function facts(): Map<string, HTMLElement> {
    const out = new Map<string, HTMLElement>();
    for (const dt of document.querySelectorAll(".warden-jobs-facts dt")) {
        out.set(dt.textContent ?? "", dt.nextElementSibling as HTMLElement);
    }
    return out;
}

const fact = (label: string): HTMLElement => {
    const dd = facts().get(label);
    if (!dd) throw new Error(`no "${label}" row; rows: ${[...facts().keys()].join(", ")}`);
    return dd;
};

const approve = () => screen.getByRole("button", { name: /^Approve$/ });
const cancel = () => screen.getByRole("button", { name: /^Cancel$/ });
const pills = (el: Element) => [...el.querySelectorAll(".ui-pill")] as HTMLElement[];
const tone = (el: Element) => /ui-pill--(\w+)/.exec(el.className)?.[1];

describe("publisherNote", () => {
    const sig = (over: Record<string, unknown>) =>
        job(
            {},
            {
                source: "installed",
                signature: {
                    state: "signed",
                    publisher: "acme",
                    fingerprint: "FP-1",
                    pinned: "FP-0",
                    ...over,
                } as never,
            }
        );

    it("says a built-in template is AgentMux's, whatever its signature says", () => {
        const info = mkTemplate({
            source: "builtin",
            signature: { state: "key_changed", publisher: "agentmux", fingerprint: "x", pinned: "y" },
        });
        expect(publisherNote(info)).toEqual({ text: "Published by AgentMux and part of this app.", tone: "ok" });
    });

    it("says a signed one is signed with the pinned key", () => {
        expect(publisherNote(sig({ state: "signed" }))).toEqual({
            text: "Signed with the key pinned for acme (FP-1).",
            tone: "ok",
        });
    });

    it("says a new publisher is new and what approving pins", () => {
        expect(publisherNote(sig({ state: "signed_new" }))).toEqual({
            text: "acme is new to you. Approving pins its key FP-1.",
            tone: "warn",
        });
    });

    it("says a changed key differs, with both keys", () => {
        expect(publisherNote(sig({ state: "key_changed" }))).toEqual({
            text: "The key differs from the one pinned for acme: pinned FP-0, this one FP-1.",
            tone: "error",
        });
    });

    it("names a missing pin and a missing key in a changed-key note", () => {
        expect(publisherNote(sig({ state: "key_changed", pinned: null, fingerprint: null })).text).toBe(
            "The key differs from the one pinned for acme: pinned none, this one unsigned."
        );
    });

    it("says an unsigned one is unsigned", () => {
        expect(publisherNote(sig({ state: "unsigned" }))).toEqual({
            text: "Unsigned. Nothing vouches that acme wrote it.",
            tone: "warn",
        });
    });
});

describe("reporterNote", () => {
    it("a new reporter: approving pins its key and accepts its reports", () => {
        expect(reporterNote(pin("new"))).toEqual({
            text: "New to you. Approving pins its key K7Q2-MZ4D-PX3A-9TWE and accepts the signed run reports it sends for this job at once.",
            tone: "warn",
            block: false,
        });
    });

    it("a pinned reporter is fine", () => {
        expect(reporterNote(pin("pinned"))).toEqual({
            text: "Already pinned (K7Q2-MZ4D-PX3A-9TWE).",
            tone: "ok",
            block: false,
        });
    });

    it("a name pinned to another key is refused", () => {
        expect(reporterNote(pin("name_taken"))).toEqual({
            text: "The name acme-ci is pinned to a different key than this one (K7Q2-MZ4D-PX3A-9TWE). Forget that reporter first; approving is refused until then.",
            tone: "error",
            block: true,
        });
    });

    it("a key pinned under another name is refused", () => {
        expect(reporterNote(pin("key_taken"))).toEqual({
            text: "This key (K7Q2-MZ4D-PX3A-9TWE) is already pinned under another name. Approving is refused.",
            tone: "error",
            block: true,
        });
    });
});

describe("the heading, the description and the publisher row", () => {
    it("is a section labelled with the template id", () => {
        show(job());
        expect(screen.getByRole("region", { name: "Approve agentmux.weekly-report" })).toHaveClass(
            "warden-jobs-approval"
        );
    });

    it("asks to approve the template by name and version", () => {
        show(job({ name: "Weekly report", version: "2.3.4" }));
        expect(screen.getByRole("heading", { level: 3 })).toHaveTextContent("Approve Weekly report 2.3.4?");
    });

    it("shows the description", () => {
        show(job({ description: "Reads repositories since the last run and reports." }));
        expect(screen.getByText("Reads repositories since the last run and reports.")).toHaveClass(
            "warden-manager-dim"
        );
    });

    it("shows the publisher as a pill in the note's tone, with the note beside it", () => {
        show(
            job(
                {},
                {
                    source: "installed",
                    signature: { state: "signed_new", publisher: "acme", fingerprint: "FP-1", pinned: null },
                }
            )
        );
        const [pill] = pills(fact("Publisher"));
        expect(pill).toHaveTextContent("acme");
        expect(tone(pill)).toBe("warn");
        expect(fact("Publisher").textContent).toBe("acme acme is new to you. Approving pins its key FP-1.");
    });

    it("shows an AgentMux template's publisher in the ok tone", () => {
        show(job());
        const [pill] = pills(fact("Publisher"));
        expect(pill).toHaveTextContent("agentmux");
        expect(tone(pill)).toBe("ok");
        expect(fact("Publisher")).toHaveTextContent("Published by AgentMux and part of this app.");
    });

    it("shows a changed key in the error tone", () => {
        show(
            job(
                {},
                {
                    source: "installed",
                    signature: { state: "key_changed", publisher: "acme", fingerprint: "FP-1", pinned: "FP-0" },
                }
            )
        );
        expect(tone(pills(fact("Publisher"))[0])).toBe("error");
    });
});

describe("a job template's facts", () => {
    it("shows Triggers, Does, Outputs, Secrets and Default caps, and no cloud row", () => {
        show(job());
        expect([...facts().keys()]).toEqual(["Publisher", "Triggers", "Does", "Outputs", "Secrets", "Default caps"]);
    });

    describe("Triggers", () => {
        const triggers = (trigger: Record<string, unknown> | null | undefined) => {
            show(job({ trigger }));
            return fact("Triggers").textContent;
        };

        it("a cron trigger is described in its zone", () => {
            expect(triggers({ kind: "cron", cron: "30 7 * * *", tz: "Europe/Oslo" })).toBe("daily 07:30 Europe/Oslo");
        });

        it("a cron trigger without a zone is in UTC", () => {
            expect(triggers({ kind: "cron", cron: "30 7 * * *" })).toBe("daily 07:30 UTC");
        });

        it("a cron trigger whose zone is not text is in UTC", () => {
            expect(triggers({ kind: "cron", cron: "30 7 * * *", tz: 5 })).toBe("daily 07:30 UTC");
        });

        it("a cron trigger with no expression is just on a schedule", () => {
            expect(triggers({ kind: "cron" })).toBe("on a schedule");
        });

        it("a cron trigger whose expression is not text is just on a schedule", () => {
            expect(triggers({ kind: "cron", cron: 5 })).toBe("on a schedule");
        });

        it("an interval is in minutes", () => {
            expect(triggers({ kind: "every", seconds: 900 })).toBe("every 15 min");
        });

        it("an interval is rounded to whole minutes", () => {
            expect(triggers({ kind: "every", seconds: 90 })).toBe("every 2 min");
            expect(triggers({ kind: "every", seconds: 89 })).toBe("every 1 min");
        });

        it("an interval without a number of seconds is just on an interval", () => {
            expect(triggers({ kind: "every" })).toBe("on an interval");
            cleanup();
            expect(triggers({ kind: "every", seconds: "900" })).toBe("on an interval");
        });

        it("a one-off is once", () => {
            expect(triggers({ kind: "once" })).toBe("once");
        });

        it("an event trigger is on an event", () => {
            expect(triggers({ kind: "event" })).toBe("on an event");
        });

        it("any other kind is by hand", () => {
            expect(triggers({ kind: "manual" })).toBe("by hand");
            cleanup();
            expect(triggers({})).toBe("by hand");
        });

        it("no trigger is run by hand", () => {
            expect(triggers(null)).toBe("none (run by hand)");
            cleanup();
            expect(triggers(undefined)).toBe("none (run by hand)");
        });
    });

    describe("Does", () => {
        it("names the action's kind", () => {
            show(job({ action: { kind: "run" } }));
            expect(fact("Does")).toHaveTextContent("run an agent with the template's brief");
        });

        it("defaults to message when the action has no kind", () => {
            show(job({ action: {} }));
            expect(fact("Does")).toHaveTextContent("message an agent with the template's brief");
        });

        it("defaults to message when there is no action", () => {
            show(job({ action: null }));
            expect(fact("Does")).toHaveTextContent("message an agent with the template's brief");
        });

        it("says when the job reads text nobody vouches for", () => {
            show(job({ readsUntrusted: true }));
            expect(screen.getByText(/reads text nobody vouches for/)).toHaveClass("warden-manager-dim");
        });

        it("says nothing about it otherwise", () => {
            show(job({ readsUntrusted: false }));
            expect(screen.queryByText(/reads text nobody vouches for/)).toBeNull();
        });
    });

    describe("Default caps", () => {
        const caps = (policy: Record<string, unknown> | null | undefined) => {
            show(job({ policy }));
            return fact("Default caps").textContent;
        };

        it("lists each cap that is a number", () => {
            expect(caps({ caps: { run_usd: 3, day_usd: 20, messages_per_day: 50 } })).toBe(
                "$3 per run, $20 per day, 50 messages per day"
            );
        });

        it("lists one cap alone", () => {
            expect(caps({ caps: { run_usd: 3 } })).toBe("$3 per run");
            cleanup();
            expect(caps({ caps: { day_usd: 20 } })).toBe("$20 per day");
            cleanup();
            expect(caps({ caps: { messages_per_day: 50 } })).toBe("50 messages per day");
        });

        it("keeps a cap of zero", () => {
            expect(caps({ caps: { run_usd: 0 } })).toBe("$0 per run");
        });

        it("skips a cap that is not a number", () => {
            expect(caps({ caps: { run_usd: "3", day_usd: 20 } })).toBe("$20 per day");
        });

        it("says the defaults when there are no caps", () => {
            expect(caps({ caps: {} })).toBe("the defaults");
            cleanup();
            expect(caps({})).toBe("the defaults");
            cleanup();
            expect(caps(null)).toBe("the defaults");
            cleanup();
            expect(caps(undefined)).toBe("the defaults");
        });
    });
});

describe("a cloud template's facts", () => {
    it("shows Owner, Reporter, Expects, Run limit and Run now between Publisher and Outputs, and no job row", () => {
        show(cloud());
        expect([...facts().keys()]).toEqual([
            "Publisher",
            "Owner",
            "Reporter",
            "Expects",
            "Run limit",
            "Run now",
            "Outputs",
            "Secrets",
        ]);
    });

    it("shows the owner", () => {
        show(cloud({ owner: "ops-team" }));
        expect(fact("Owner")).toHaveTextContent(/^ops-team$/);
    });

    it("a new reporter is a warn pill with the new-reporter note", () => {
        show(cloud({}, { reporter: pin("new") }));
        const [pill] = pills(fact("Reporter"));
        expect(pill).toHaveTextContent("acme-ci");
        expect(tone(pill)).toBe("warn");
        expect(fact("Reporter").textContent).toMatch(
            /^acme-ci New to you\. Approving pins its key K7Q2-MZ4D-PX3A-9TWE/
        );
    });

    it("a pinned reporter is an ok pill", () => {
        show(cloud({}, { reporter: pin("pinned") }));
        expect(tone(pills(fact("Reporter"))[0])).toBe("ok");
        expect(fact("Reporter")).toHaveTextContent("Already pinned (K7Q2-MZ4D-PX3A-9TWE).");
    });

    it("a refused reporter is an error pill", () => {
        show(cloud({}, { reporter: pin("name_taken") }));
        expect(tone(pills(fact("Reporter"))[0])).toBe("error");
        expect(fact("Reporter")).toHaveTextContent("approving is refused until then");
    });

    it("shows the reporter's name from the note, not from the manifest", () => {
        show(
            cloud(
                { reporter: { name: "manifest-name", key: "AAAA" } },
                { reporter: pin("pinned", { name: "note-name" }) }
            )
        );
        expect(pills(fact("Reporter"))[0]).toHaveTextContent("note-name");
    });

    it("says none named without a reporter note", () => {
        show(cloud());
        const dim = fact("Reporter").querySelector(".warden-manager-dim");
        expect(dim).toHaveTextContent("none named");
        expect(pills(fact("Reporter"))).toHaveLength(0);
    });

    describe("Expects", () => {
        it("a schedule and how long until a report is missed", () => {
            show(cloud({ expect: { cron: "0 2 * * *", tz: "UTC", graceMinutes: 30 } }));
            expect(fact("Expects")).toHaveTextContent(/^daily 02:00 UTC, a report is missed 30 min after its slot$/);
        });

        it("in the schedule's own zone", () => {
            show(cloud({ expect: { cron: "30 7 * * *", tz: "Europe/Oslo", graceMinutes: 5 } }));
            expect(fact("Expects")).toHaveTextContent(
                /^daily 07:30 Europe\/Oslo, a report is missed 5 min after its slot$/
            );
        });

        it("none means events start it", () => {
            show(cloud({ expect: "none" }));
            expect(fact("Expects")).toHaveTextContent(/^no schedule: events start it$/);
        });

        it("no expectation at all reads the same", () => {
            show(cloud({ expect: null }));
            expect(fact("Expects")).toHaveTextContent(/^no schedule: events start it$/);
            cleanup();
            show(cloud({ expect: undefined }));
            expect(fact("Expects")).toHaveTextContent(/^no schedule: events start it$/);
        });
    });

    it("shows the run limit in minutes", () => {
        show(cloud({ maxDurationMinutes: 45 }));
        expect(fact("Run limit")).toHaveTextContent(/^45 min before a run counts as lost$/);
    });

    describe("Run now", () => {
        it("opens the link when there is one", () => {
            show(cloud({ runNow: { link: "https://ci.example/run" } }));
            expect(fact("Run now")).toHaveTextContent(/^opens https:\/\/ci\.example\/run$/);
        });

        it("prefers the link to the instruction when there are both", () => {
            show(cloud({ runNow: { link: "https://ci.example/run", askOwner: "Run it" } }));
            expect(fact("Run now")).toHaveTextContent(/^opens https:\/\/ci\.example\/run$/);
        });

        it("asks the owner otherwise", () => {
            show(cloud({ owner: "ops", runNow: { askOwner: "Run the nightly workflow" } }));
            expect(fact("Run now")).toHaveTextContent(/^asks ops: Run the nightly workflow$/);
        });

        it("asks the owner when the link is empty", () => {
            show(cloud({ owner: "ops", runNow: { link: "", askOwner: "Run it" } }));
            expect(fact("Run now")).toHaveTextContent(/^asks ops: Run it$/);
        });

        it("has no row without run-now", () => {
            show(cloud({ runNow: null }));
            expect(facts().has("Run now")).toBe(false);
            expect(screen.queryByText("Run now")).toBeNull();
        });
    });

    it("shows a cloud template's outputs and secrets like a job's", () => {
        show(cloud({ outputs: ["report_file"], secrets: [{ name: "CI_TOKEN", param: "token" }] }));
        expect(pills(fact("Outputs"))).toHaveLength(1);
        expect(pills(fact("Secrets"))).toHaveLength(1);
    });
});

describe("Outputs", () => {
    it("says none when there are none", () => {
        show(job({ outputs: [] }));
        expect(fact("Outputs")).toHaveTextContent(/^none$/);
        expect(fact("Outputs").querySelector(".warden-manager-dim")).not.toBeNull();
        expect(pills(fact("Outputs"))).toHaveLength(0);
    });

    it("shows one pill per output, in order", () => {
        show(job({ outputs: ["report_file", "pull_request", "memory"] }));
        expect(pills(fact("Outputs")).map((p) => p.textContent)).toEqual(["report_file", "pull_request", "memory"]);
        expect(fact("Outputs")).not.toHaveTextContent("none");
    });

    it("warns about an output that writes outside AgentMux, and says so in its tooltip", () => {
        show(job({ outputs: ["pull_request", "comment", "message:others", "deploy"] }));
        for (const p of pills(fact("Outputs"))) {
            expect(tone(p), p.textContent ?? "").toBe("warn");
            expect(p).toHaveAttribute("title", "Writes outside AgentMux");
        }
    });

    it("warns about the outside outputs by their name before the colon", () => {
        show(job({ outputs: ["comment:issue-12", "deploy:prod", "pull_request:acme/repo"] }));
        for (const p of pills(fact("Outputs"))) {
            expect(tone(p), p.textContent ?? "").toBe("warn");
            expect(p).toHaveAttribute("title", "Writes outside AgentMux");
        }
    });

    it("keeps an output that stays inside AgentMux neutral and without a tooltip", () => {
        show(job({ outputs: ["report_file", "memory", "message:self", "message"] }));
        for (const p of pills(fact("Outputs"))) {
            expect(tone(p), p.textContent ?? "").toBe("neutral");
            expect(p).not.toHaveAttribute("title");
        }
    });

    it("does not warn about a name that only contains an outside one", () => {
        show(job({ outputs: ["deployment", "commentary"] }));
        for (const p of pills(fact("Outputs"))) expect(tone(p), p.textContent ?? "").toBe("neutral");
    });
});

describe("Secrets", () => {
    it("says none when there are none", () => {
        show(job({ secrets: [] }));
        expect(fact("Secrets")).toHaveTextContent(/^none$/);
        expect(fact("Secrets").querySelector(".warden-manager-dim")).not.toBeNull();
        expect(pills(fact("Secrets"))).toHaveLength(0);
    });

    it("shows one neutral pill per secret by name, in order", () => {
        show(
            job({
                secrets: [
                    { name: "REPO_TOKEN", param: "token" },
                    { name: "DB_URL", param: "db" },
                ],
            })
        );
        const ps = pills(fact("Secrets"));
        expect(ps.map((p) => p.textContent)).toEqual(["REPO_TOKEN", "DB_URL"]);
        expect(ps.map(tone)).toEqual(["neutral", "neutral"]);
        expect(fact("Secrets")).not.toHaveTextContent("none");
    });

    it("explains a secret in its tooltip when it has a description", () => {
        show(job({ secrets: [{ name: "REPO_TOKEN", param: "token", description: "Reads the repository" }] }));
        expect(pills(fact("Secrets"))[0]).toHaveAttribute("title", "Reads the repository");
    });

    it("has no tooltip without a description", () => {
        show(
            job({
                secrets: [
                    { name: "A", param: "a" },
                    { name: "B", param: "b", description: null },
                ],
            })
        );
        for (const p of pills(fact("Secrets"))) expect(p).not.toHaveAttribute("title");
    });
});

describe("the footnote", () => {
    it("tells a job template's operator that approving runs nothing", () => {
        show(job());
        expect(
            screen.getByText(
                "Approving runs nothing. It lets you make jobs from exactly this version; a changed version asks again."
            )
        ).toBeInTheDocument();
        expect(screen.queryByText(/Approving starts nothing/)).toBeNull();
    });

    it("tells a cloud template's operator that it pins the key and starts nothing", () => {
        show(cloud());
        expect(
            screen.getByText(
                /Approving starts nothing: AgentMux holds no cloud credentials\. It pins the reporter's\s+key and lets this job's signed run reports into the Jobs ledger; a changed version asks\s+again\./
            )
        ).toBeInTheDocument();
        expect(screen.queryByText(/Approving runs nothing/)).toBeNull();
    });
});

describe("a template that can't be read", () => {
    const broken = unreadable;

    it("shows srv's reason in place of the facts", () => {
        show(broken("manifest.json: missing field `id`"));
        const err = screen.getByText("manifest.json: missing field `id`");
        expect(err).toHaveClass("warden-manager-error");
        expect(document.querySelector(".warden-jobs-facts")).toBeNull();
        expect(screen.queryByRole("heading")).toBeNull();
    });

    it("says it can't be read when srv gave no reason", () => {
        show(broken(null));
        expect(screen.getByText("This template can't be read.")).toHaveClass("warden-manager-error");
    });

    it("shows no footnote", () => {
        show(broken(null));
        expect(screen.queryByText(/Approving (runs|starts) nothing/)).toBeNull();
    });

    it("cannot be approved but can be cancelled", () => {
        const { onApprove, onCancel } = show(broken(null));
        expect(approve()).toBeDisabled();
        fireEvent.click(approve());
        expect(onApprove).not.toHaveBeenCalled();
        expect(cancel()).not.toBeDisabled();
        fireEvent.click(cancel());
        expect(onCancel).toHaveBeenCalledTimes(1);
    });
});

describe("the error", () => {
    it("shows an alert with the message", () => {
        show(job(), { error: "Approving failed: key changed" });
        const alert = screen.getByRole("alert");
        expect(alert).toHaveTextContent("Approving failed: key changed");
        expect(alert).toHaveClass("warden-manager-error");
    });

    it("shows nothing without one", () => {
        show(job(), { error: null });
        expect(screen.queryByRole("alert")).toBeNull();
    });

    it("shows nothing for an empty message", () => {
        show(job(), { error: "" });
        expect(screen.queryByRole("alert")).toBeNull();
    });

    it("sits between the facts and the buttons", () => {
        show(job(), { error: "no" });
        const section = screen.getByRole("region");
        const kids = [...section.children].map((c) => c.className);
        expect(kids.indexOf("warden-manager-error")).toBeGreaterThan(kids.indexOf("warden-manager-dim"));
        expect(kids.indexOf("warden-manager-error")).toBeLessThan(kids.indexOf("warden-jobs-actions"));
    });
});

describe("Approve and Cancel", () => {
    it("Approve is the accent button with a check, and Cancel is quiet", () => {
        show(job());
        expect(approve()).toHaveClass("ui-tone-accent");
        expect(approve().querySelector(".fa-check")).not.toBeNull();
        expect(cancel()).toHaveClass("ui-tone-quiet");
        expect(approve().parentElement).toHaveClass("warden-jobs-actions");
        expect(cancel().parentElement).toHaveClass("warden-jobs-actions");
    });

    it("Approve calls onApprove with no arguments, once", () => {
        const { onApprove, onCancel } = show(job());
        fireEvent.click(approve());
        expect(onApprove).toHaveBeenCalledTimes(1);
        expect(onApprove).toHaveBeenCalledWith();
        expect(onCancel).not.toHaveBeenCalled();
    });

    it("Cancel calls onCancel with no arguments, once", () => {
        const { onApprove, onCancel } = show(job());
        fireEvent.click(cancel());
        expect(onCancel).toHaveBeenCalledTimes(1);
        expect(onCancel).toHaveBeenCalledWith();
        expect(onApprove).not.toHaveBeenCalled();
    });

    it("while busy, Approve shows a spinner and both buttons are off", () => {
        const { onApprove, onCancel } = show(job(), { busy: true });
        expect(approve()).toBeDisabled();
        expect(approve()).toHaveAttribute("aria-busy", "true");
        expect(approve().querySelector(".fa-spinner")).not.toBeNull();
        expect(cancel()).toBeDisabled();
        fireEvent.click(approve());
        fireEvent.click(cancel());
        expect(onApprove).not.toHaveBeenCalled();
        expect(onCancel).not.toHaveBeenCalled();
    });

    it("when idle, neither button is off", () => {
        show(job());
        expect(approve()).not.toBeDisabled();
        expect(approve()).not.toHaveAttribute("aria-busy");
        expect(cancel()).not.toBeDisabled();
    });

    it("a reporter whose pin would be refused turns Approve off but not Cancel", () => {
        for (const state of ["name_taken", "key_taken"] as const) {
            const { onApprove, onCancel } = show(cloud({}, { reporter: pin(state) }));
            expect(approve()).toBeDisabled();
            expect(cancel()).not.toBeDisabled();
            fireEvent.click(approve());
            fireEvent.click(cancel());
            expect(onApprove).not.toHaveBeenCalled();
            expect(onCancel).toHaveBeenCalledTimes(1);
            cleanup();
        }
    });

    it("a reporter that is new or pinned leaves Approve on", () => {
        for (const state of ["new", "pinned"] as const) {
            show(cloud({}, { reporter: pin(state) }));
            expect(approve()).not.toBeDisabled();
            cleanup();
        }
    });

    it("a refused reporter on a job template refuses it too", () => {
        show(job({}, { reporter: pin("name_taken") }));
        expect(approve()).toBeDisabled();
    });

    it("no reporter leaves Approve on", () => {
        show(cloud({}, { reporter: undefined }));
        expect(approve()).not.toBeDisabled();
    });
});

describe("it follows its props", () => {
    it("turns the buttons off and on with busy, and shows and hides the error", () => {
        const [busy, setBusy] = createSignal(false);
        const [error, setError] = createSignal<string | null>(null);
        render(() => (
            <TemplateApproval info={job()} busy={busy()} error={error()} onApprove={() => {}} onCancel={() => {}} />
        ));
        expect(approve()).not.toBeDisabled();
        expect(screen.queryByRole("alert")).toBeNull();

        setBusy(true);
        setError("Approving failed");
        expect(approve()).toBeDisabled();
        expect(cancel()).toBeDisabled();
        expect(screen.getByRole("alert")).toHaveTextContent("Approving failed");

        setBusy(false);
        setError(null);
        expect(approve()).not.toBeDisabled();
        expect(cancel()).not.toBeDisabled();
        expect(screen.queryByRole("alert")).toBeNull();
    });

    it("shows the new template when the info changes", () => {
        const [info, setInfo] = createSignal(job());
        render(() => (
            <TemplateApproval info={info()} busy={false} error={null} onApprove={() => {}} onCancel={() => {}} />
        ));
        expect(screen.getByRole("heading")).toHaveTextContent("Approve Weekly report 1.0.0?");

        setInfo(cloud({ name: "Nightly build", version: "3.0.0" }, { reporter: pin("name_taken") }));
        expect(screen.getByRole("heading")).toHaveTextContent("Approve Nightly build 3.0.0?");
        expect(fact("Owner")).toHaveTextContent("ops");
        expect(approve()).toBeDisabled();

        setInfo(unreadable("gone"));
        expect(screen.getByText("gone")).toBeInTheDocument();
        expect(screen.queryByRole("heading")).toBeNull();
        expect(approve()).toBeDisabled();
    });
});


describe("a Drone template's facts", () => {
    const drone = (summary: { nodes: number; agents: number; writes_outside: string[] }, manifest: Partial<JobTemplateManifest> = {}) =>
        job({ kind: "drone", action: null, outputs: [], ...manifest }, { drone: summary });

    it("shows the graph's size, the agents it starts and the calls it makes outside AgentMux", () => {
        show(drone({ nodes: 5, agents: 2, writes_outside: ["post: POST https://example.com/hook"] }));
        expect(fact("Does").textContent).toBe("runs a drone of 5 nodes, 2 of them start an agent");
        const outside = pills(fact("Calls outside"));
        expect(outside.map((p) => p.textContent)).toEqual(["post: POST https://example.com/hook"]);
        expect(tone(outside[0])).toBe("warn");
    });

    it("says none when it makes no call outside, and never describes a brief", () => {
        show(drone({ nodes: 1, agents: 0, writes_outside: [] }));
        expect(fact("Does").textContent).toBe("runs a drone of 1 node");
        expect(fact("Calls outside").textContent).toBe("none");
        expect(document.body.textContent).not.toContain("template's brief");
    });
});
