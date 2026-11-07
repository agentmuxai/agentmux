// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * JobUpdateDiff (a job's offered template update), pinned as a whole ahead of splitting it into a state object and
 * view parts: each scenario mounts the real component over a stand-in model whose answers the scenario controls, with
 * stand-ins for the kit's button and pill and for the approval prompt, clicks its buttons, and logs the component as
 * an indented tree and, in order, every call it makes (the diff it asks for, the save, the template it opens, the
 * approval). The logs are snapshots recorded on the unsplit component; the split must leave them byte for byte.
 * (warden-jobs-manager.test.tsx asserts what the person sees through the manager; this pins the markup and the calls.)
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { type JSX } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { TemplateUpdateDiff } from "@/types/rpc/TemplateUpdateDiff";

const T: string[] = [];
const log = (line: string): void => {
    T.push(line);
};

const h = vi.hoisted(() => ({
    diff: null as any,
    save: null as any,
    detail: null as any,
    approve: null as any,
    approval: null as any,
}));

vi.mock("@/app/element/ui", () => ({
    Button: (p: { tone?: string; icon?: string; busy?: boolean; onClick?: () => void; children?: JSX.Element }) => (
        <span
            data-stand="Button"
            data-tone={p.tone}
            data-icon={p.icon}
            data-busy={String(p.busy)}
            onClick={() => (log(`Button ${JSON.stringify(p.children)} onClick`), p.onClick?.())}
        >
            {p.children}
        </span>
    ),
    Pill: (p: { tone?: string; children?: JSX.Element }) => (
        <span data-stand="Pill" data-tone={p.tone}>
            {p.children}
        </span>
    ),
}));
vi.mock("./job-template-approval", () => ({
    TemplateApproval: (p: any) => {
        h.approval = p;
        return (
            <div
                data-stand="TemplateApproval"
                data-info={p.info.id}
                data-busy={String(p.busy)}
                data-error={String(p.error)}
            />
        );
    },
}));
vi.mock("./job-editor.scss", () => ({}));

import { JobUpdateDiff } from "./job-update-diff";
import { mkJob } from "./jobs-fixtures";

const SNAP = (name: string) => `./__snapshots__/job-update-diff.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const ATTRS = /^(class|role|aria-.*|data-.*|title|style|disabled|type|value|placeholder)$/;

/** The component as an indented tree: tags, classes, the attributes that carry state, and text. */
function dump(node: Node, depth = 0, out: string[] = []): string[] {
    const pad = "  ".repeat(depth);
    if (node.nodeType === Node.TEXT_NODE) {
        const t = (node.textContent ?? "").trim();
        if (t) out.push(`${pad}"${t}"`);
        return out;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return out;
    const el = node as HTMLElement;
    const attrs = [...el.attributes]
        .filter((a) => ATTRS.test(a.name))
        .map((a) => `${a.name}=${JSON.stringify(a.value)}`);
    out.push(`${pad}<${el.tagName.toLowerCase()}${attrs.length ? " " + attrs.join(" ") : ""}>`);
    el.childNodes.forEach((c) => dump(c, depth + 1, out));
    return out;
}

const diffOf = (over: Partial<TemplateUpdateDiff> = {}): TemplateUpdateDiff => ({
    job_id: "j1",
    template_id: "nightly-build",
    status: "available",
    reason: null,
    from_version: "1.0.0",
    to_version: "1.1.0",
    to_hash: "hash-new",
    brief: null,
    outputs: null,
    secrets: null,
    policy: [],
    params_error: null,
    ...over,
});
const FULL = diffOf({
    brief: { before: "Build it.", after: "Build it and test it." },
    outputs: { added: ["comment"], removed: ["pull_request"] },
    secrets: { added: ["REPO_TOKEN"], removed: [] },
    policy: [
        { field: "max_runs_per_day", before: "4", after: "8" },
        { field: "timeout", before: "10m", after: "5m" },
    ],
} as any);
const INFO = { id: "nightly-build", hash: "hash-new", signature: { fingerprint: "ab:cd" } };

let host: HTMLElement;
const onClose = vi.fn(() => log("onClose"));

const look = (label: string) => {
    log(`-- ${label}`);
    log(dump(host).join("\n") || "-");
};
const flush = async () => {
    for (let i = 0; i < 12; i++) await Promise.resolve();
};
const click = (text: string) => {
    const b = [...host.querySelectorAll<HTMLElement>("[data-stand='Button']")].find((x) => x.textContent?.trim() === text);
    fireEvent.click(b!);
};
const defer = <T,>() => {
    let resolve!: (v: T) => void;
    let reject!: (e: unknown) => void;
    const promise = new Promise<T>((res, rej) => ((resolve = res), (reject = rej)));
    return { promise, resolve, reject };
};

async function mount(diff: () => Promise<TemplateUpdateDiff> = async () => FULL) {
    h.diff = diff;
    h.save = async () => ({ ok: true, job_id: "j1" });
    h.detail = async () => ({ template: INFO });
    h.approve = async () => ({});
    const model = {
        updateDiff: (id: string) => (log(`model.updateDiff ${id}`), h.diff()),
        save: (req: unknown) => (log(`model.save ${JSON.stringify(req)}`), h.save()),
        templateDetail: (id: string) => (log(`model.templateDetail ${id}`), h.detail()),
        approveTemplate: (req: unknown) => (log(`model.approveTemplate ${JSON.stringify(req)}`), h.approve()),
    };
    const view = render(() => <JobUpdateDiff job={mkJob()} model={model as any} onClose={onClose} />);
    host = view.container;
    await flush();
}

beforeEach(() => {
    T.length = 0;
    onClose.mockClear();
});
afterEach(() => cleanup());

describe("JobUpdateDiff, pinned", () => {
    it("says it is loading until the diff arrives, and shows the load's error (and no loading line) when it fails", async () => {
        const d = defer<TemplateUpdateDiff>();
        await mount(() => d.promise);
        look("loading");
        d.resolve(FULL);
        await flush();
        look("loaded");
        cleanup();
        T.push("-- load fails");
        await mount(() => Promise.reject(new Error("srv is down")));
        look("failed");
        return snapshot("load");
    });

    it("shows what the update changes: the brief before and after, the outputs and secrets added and removed, and each policy field", async () => {
        await mount();
        look("a full update");
        cleanup();
        T.push("-- nothing the job holds changes");
        await mount(async () => diffOf());
        look("available, nothing changed");
        cleanup();
        T.push("-- only some parts");
        await mount(async () => diffOf({ outputs: { added: [], removed: ["deploy"] }, policy: [{ field: "timeout", before: "1m", after: "2m" }] } as any));
        look("outputs and one policy field");
        return snapshot("available");
    });

    it("explains a job already at the template's version, an unavailable update, and one that needs approval first (with its review button), and refuses to accept when the new version can't take the job's settings", async () => {
        await mount(async () => diffOf({ status: "none" }));
        look("at the template's version");
        cleanup();
        T.push("-- unavailable");
        await mount(async () => diffOf({ status: "unavailable", reason: "The template was removed." }));
        look("unavailable");
        cleanup();
        T.push("-- needs approval");
        await mount(async () => diffOf({ status: "needs_approval", reason: "Version 1.1.0 isn't approved yet." }));
        look("needs approval");
        cleanup();
        T.push("-- the new version can't take the settings");
        await mount(async () => diffOf({ params_error: "branch is required", brief: { before: "a", after: "b" } } as any));
        look("params error");
        return snapshot("statuses");
    });

    it("accepts the update: sends exactly the shown version, shows busy while it is out, closes on success, and says why when it isn't saved or throws", async () => {
        await mount();
        const save = defer<any>();
        h.save = () => save.promise;
        click("Accept the update");
        look("saving");
        save.resolve({ ok: true, job_id: "j1" });
        await flush();
        log(`closed ${onClose.mock.calls.length} time(s)`);
        look("saved");
        h.save = async () => ({ ok: false, job_id: "j1", error: "The name is taken." });
        click("Accept the update");
        await flush();
        look("not saved, with a reason");
        h.save = async () => ({ ok: false, job_id: "j1" });
        click("Accept the update");
        await flush();
        look("not saved, no reason");
        h.save = async () => {
            throw new Error("offline");
        };
        click("Accept the update");
        await flush();
        look("save threw");
        click("Close");
        return snapshot("accept");
    });

    it("opens the unapproved version for review, approves it with the signer's fingerprint (or none), reloads the diff, and returns to the diff on cancel or after a failed look", async () => {
        const diffs = [diffOf({ status: "needs_approval", reason: "Needs approval." }), diffOf()];
        await mount(async () => diffs.shift() ?? diffOf());
        click("Review and approve 1.1.0");
        await flush();
        look("approval prompt");
        log(`approval props: info=${h.approval.info.id} busy=${h.approval.busy} error=${h.approval.error}`);
        const approve = defer<any>();
        h.approve = () => approve.promise;
        h.approval.onApprove();
        await flush();
        log(`while approving: busy=${h.approval.busy}`);
        approve.resolve({});
        await flush();
        look("approved, diff reloaded");
        cleanup();
        T.push("-- cancel the review");
        await mount(async () => diffOf({ status: "needs_approval", reason: "Needs approval." }));
        click("Review and approve 1.1.0");
        await flush();
        h.approval.onCancel();
        look("back at the diff after cancel");
        cleanup();
        T.push("-- approval fails, unsigned template");
        h.detail = async () => ({ template: { ...INFO, signature: {} } });
        await mount(async () => diffOf({ status: "needs_approval", reason: "Needs approval." }));
        h.detail = async () => ({ template: { ...INFO, signature: {} } });
        click("Review and approve 1.1.0");
        await flush();
        h.approve = async () => {
            throw new Error("key mismatch");
        };
        h.approval.onApprove();
        await flush();
        log(`approval props after failure: busy=${h.approval.busy} error=${h.approval.error}`);
        cleanup();
        T.push("-- looking at the template fails");
        await mount(async () => diffOf({ status: "needs_approval", reason: "Needs approval." }));
        h.detail = async () => {
            throw new Error("no such template");
        };
        click("Review and approve 1.1.0");
        await flush();
        look("template lookup failed");
        return snapshot("review");
    });
});
