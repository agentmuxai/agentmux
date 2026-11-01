// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The approval prompt of a cloud job template: it shows the owner, the reporter whose key approving pins and
// what the job is expected to do, and refuses to offer Approve when the pin would be refused
// (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §4.3).

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { JobTemplateInfo } from "@/types/rpc/JobTemplateInfo";
import type { ReporterNote } from "@/types/rpc/ReporterNote";

import { TemplateApproval } from "./job-template-approval";
import { mkTemplate } from "./jobs-fixtures";

afterEach(cleanup);

function cloud(reporter: ReporterNote | undefined, over: Record<string, unknown> = {}): JobTemplateInfo {
    const base = mkTemplate({ id: "acme.nightly", source: "installed", state: "needs_approval" });
    return {
        ...base,
        reporter,
        manifest: {
            ...base.manifest!,
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
            ...over,
        },
    };
}

const note = (state: ReporterNote["state"]): ReporterNote => ({
    name: "acme-ci",
    fingerprint: "K7Q2-MZ4D-PX3A-9TWE",
    state,
});

function show(info: JobTemplateInfo, onApprove = vi.fn()) {
    render(() => <TemplateApproval info={info} busy={false} error={null} onApprove={onApprove} onCancel={() => {}} />);
    return onApprove;
}

describe("approving a cloud job template", () => {
    it("shows the owner, the reporter key it pins, the schedule and the limit, and none of a job's trigger or caps", () => {
        show(cloud(note("new")));
        expect(screen.getByText("ops")).toBeInTheDocument();
        expect(screen.getByText("acme-ci")).toBeInTheDocument();
        expect(screen.getByText(/Approving pins its key K7Q2-MZ4D-PX3A-9TWE/)).toBeInTheDocument();
        expect(screen.getByText(/a report is missed 30 min after its slot/)).toBeInTheDocument();
        expect(screen.getByText(/120 min before a run counts as lost/)).toBeInTheDocument();
        expect(screen.getByText("asks ops: Run the nightly workflow")).toBeInTheDocument();
        expect(screen.getByText(/Approving starts nothing/)).toBeInTheDocument();
        expect(screen.queryByText("Triggers")).toBeNull();
        expect(screen.queryByText("Default caps")).toBeNull();
    });

    it("says no schedule when events start the job", () => {
        show(cloud(note("pinned"), { expect: "none", runNow: null }));
        expect(screen.getByText("no schedule: events start it")).toBeInTheDocument();
        expect(screen.getByText(/Already pinned/)).toBeInTheDocument();
        expect(screen.queryByText("Run now")).toBeNull();
    });

    it("approves a new or pinned reporter", () => {
        for (const state of ["new", "pinned"] as const) {
            const onApprove = show(cloud(note(state)));
            const approve = screen.getByRole("button", { name: /^Approve$/ });
            expect(approve).not.toBeDisabled();
            fireEvent.click(approve);
            expect(onApprove).toHaveBeenCalledTimes(1);
            cleanup();
        }
    });

    it("does not offer Approve when the name or the key is pinned to another", () => {
        for (const [state, says] of [
            ["name_taken", /pinned to a different key/],
            ["key_taken", /already pinned under another name/],
        ] as const) {
            const onApprove = show(cloud(note(state)));
            expect(screen.getByText(says)).toBeInTheDocument();
            const approve = screen.getByRole("button", { name: /^Approve$/ });
            expect(approve).toBeDisabled();
            fireEvent.click(approve);
            expect(onApprove).not.toHaveBeenCalled();
            cleanup();
        }
    });

    it("keeps a job template's prompt as it was", () => {
        show(mkTemplate({ state: "needs_approval" }));
        expect(screen.getByText("Triggers")).toBeInTheDocument();
        expect(screen.getByText("Default caps")).toBeInTheDocument();
        expect(screen.queryByText("Reporter")).toBeNull();
        expect(screen.getByText(/Approving runs nothing/)).toBeInTheDocument();
    });
});
