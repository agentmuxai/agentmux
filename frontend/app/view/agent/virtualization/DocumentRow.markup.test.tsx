// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the transcript rows that DocumentRow draws itself look like, pinned as markup: the error row,
 * the compaction, memory, day, session and resume rows, the History link and the CLI notices. The
 * rows that have a component of their own (markdown, tool, messages) are tested with it. The
 * behaviour of these rows (the login button, the copy menu, the peek) is in DocumentRow.test.tsx. A
 * refactor of the row must leave the committed snapshots as they are; regenerate with `vitest -u`
 * only for a change meant to look different.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("../components/ContainerUpdateButton", () => ({ ContainerUpdateButton: (p: { blockId: string }) => <button data-update={p.blockId}>Update</button> }));
vi.mock("../components/ContainerLimitButton", () => ({
    ContainerLimitButton: (p: { agentId: string; label?: string }) => <button data-limit={p.agentId}>{p.label ?? "Raise limit"}</button>,
}));

import { DocumentRow } from "./DocumentRow";
import type { DocumentNode, DocumentState } from "../../../agent-model/types";

afterEach(() => cleanup());

const emptyState = (): DocumentState => ({
    collapsedNodes: new Set(),
    pinnedNodes: new Set(),
    heldOpenNodes: new Set(),
    scrollPosition: 0,
    selectedNode: null,
    filter: { showThinking: true } as DocumentState["filter"],
});

const NOON = Date.UTC(2026, 10, 3, 12, 0, 0);

/** One element per line, so a snapshot diff points at the element that changed. A time of day depends on the zone and is blanked. */
const markup = (el: Element | null): string =>
    (el ? el.outerHTML : "(absent)")
        .replace(/></g, ">\n<")
        .replace(/\d{1,2}:\d{2}:\d{2} (AM|PM)/g, "<time>") + "\n";
const snap = (name: string) => `./__snapshots__/DocumentRow.markup.${name}.html`;

function mount(node: DocumentNode, extra: { onAgentErrorLogin?: () => void; onOpenHistory?: () => void } = {}) {
    const [n, setNode] = createSignal<DocumentNode>(node);
    const [state] = createSignal<DocumentState>(emptyState());
    const view = render(() => (
        <DocumentRow node={n} documentState={state} onToggleCollapse={() => {}} onTogglePin={() => {}} {...extra} />
    ));
    return { ...view, setNode, row: view.container.querySelector(".agent-document-row") };
}

describe("DocumentRow markup of the rows it draws itself", () => {
    it("an authentication error, with the login button, the copy icon and the signed-in-since mark absent", async () => {
        const v = mount({ type: "agent_error", id: "e1", code: 401, message: "Invalid authentication credentials" } as DocumentNode, { onAgentErrorLogin: () => {} });
        await expect(markup(v.row)).toMatchFileSnapshot(snap("error-auth"));
    });

    it("a resolved error and a non-HTTP error", async () => {
        const v = mount({ type: "agent_error", id: "e2", code: 0, message: "network down", resolved: true } as DocumentNode, { onAgentErrorLogin: () => {} });
        await expect(markup(v.row)).toMatchFileSnapshot(snap("error-resolved"));
    });

    it("a compaction that finished, with its trigger and duration", async () => {
        const v = mount({
            type: "context_compacted",
            id: "cc",
            tokensBefore: 100_000,
            tokensAfter: 5_000,
            timestamp: NOON,
            source: "real",
            trigger: "manual",
            durationMs: 12_345,
        } as DocumentNode);
        await expect(markup(v.row)).toMatchFileSnapshot(snap("compacted"));
    });

    it("a compaction under way", async () => {
        const v = mount({ type: "compaction_started", id: "cs", trigger: "auto", startedAt: NOON } as DocumentNode);
        await expect(markup(v.row)).toMatchFileSnapshot(snap("compaction-started"));
    });

    it("memory put back after a compaction, in the high band", async () => {
        const v = mount({
            type: "memory_reinjection",
            id: "mr",
            globalMemoryCount: 2,
            personalMemoryCount: 3,
            estimatedTokens: 450,
            perEntryTokens: [{ label: "global-note", source: "global", tokens: 100, sizeBytes: 400 }],
            totalSizeBytes: { global: 800, personal: 4200 },
            sizeBand: "high",
            at: NOON,
        } as DocumentNode);
        await expect(markup(v.row)).toMatchFileSnapshot(snap("memory-reinjection"));
    });

    it("a day divider", async () => {
        const v = mount({ type: "day_divider", id: "dd", dayLabel: "Today", timestamp: NOON } as DocumentNode);
        await expect(markup(v.row)).toMatchFileSnapshot(snap("day-divider"));
    });

    it("the link to the Agent History", async () => {
        const v = mount({ type: "history_link", id: "history-link" } as DocumentNode, { onOpenHistory: () => {} });
        await expect(markup(v.row)).toMatchFileSnapshot(snap("history-link"));
    });

    it("the resume check: waiting, then each verdict", async () => {
        const base = { type: "resume_preflight", id: "resume-preflight", steps: [] };
        const pending = mount({ ...base, pending: true } as DocumentNode);
        const out = [markup(pending.row)];
        cleanup();
        const recover = mount({ ...base, pending: false, verdict: "recover" } as DocumentNode);
        out.push(markup(recover.row));
        cleanup();
        const fresh = mount({ ...base, pending: false, verdict: "fresh" } as DocumentNode, { onOpenHistory: () => {} });
        out.push(markup(fresh.row));
        await expect(out.join("----\n")).toMatchFileSnapshot(snap("resume-preflight"));
    });

    it("a session outcome: a new session built from the record, and a plain one", async () => {
        const a = mount({ type: "session_outcome", id: "so1", outcome: "fresh", attemptedSid: "sid-a", actualSid: "sid-b", timestamp: NOON, continued: true } as DocumentNode);
        const out = [markup(a.row)];
        cleanup();
        const b = mount({ type: "session_outcome", id: "so2", outcome: "fresh", attemptedSid: "", timestamp: NOON } as DocumentNode);
        out.push(markup(b.row));
        await expect(out.join("----\n")).toMatchFileSnapshot(snap("session-outcome"));
    });

    it("CLI notices: an install, a container image that is out of date, and the container settings", async () => {
        const notice = (over: Record<string, unknown>) => ({ type: "cli_notice", id: "n", provider: "claude", timestamp: NOON, ...over }) as DocumentNode;
        const out: string[] = [];
        for (const n of [
            notice({ kind: "install", version: "2.1.0", state: "installed", seconds: 12 }),
            notice({ kind: "container_image", imageState: "outdated", blockId: "blk-1", running: "1", current: "2" }),
            notice({ kind: "container_caches", agentId: "ag-1", agentName: "Korp" }),
        ]) {
            out.push(markup(mount(n).row));
            cleanup();
        }
        await expect(out.join("----\n")).toMatchFileSnapshot(snap("cli-notice"));
    });
});

describe("DocumentRow rows follow their node as it is updated", () => {
    it("a CLI notice moves from installing to installed in place", () => {
        const install = (state: string) =>
            ({ type: "cli_notice", id: "n", provider: "claude", kind: "install", version: "2.1.0", state, timestamp: NOON }) as DocumentNode;
        const v = mount(install("installing"));
        const before = v.container.querySelector(".agent-cli-notice-label")!.textContent;
        v.setNode(install("installed"));
        expect(v.container.querySelector(".agent-cli-notice-label")!.textContent).not.toBe(before);
    });

    it("a compaction row shows the new numbers when its node is replaced", () => {
        const node = (tokensAfter: number) =>
            ({ type: "context_compacted", id: "cc", tokensBefore: 100_000, tokensAfter, timestamp: NOON, source: "real", trigger: "auto" }) as DocumentNode;
        const v = mount(node(5_000));
        const before = v.container.querySelector(".agent-context-compacted-detail")!.textContent;
        v.setNode(node(9_000));
        expect(v.container.querySelector(".agent-context-compacted-detail")!.textContent).not.toBe(before);
    });
});
