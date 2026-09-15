// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        NativeMemoryHistoryCommand: vi.fn(),
        NativeMemoryDiffCommand: vi.fn(),
        NativeMemoryRevertCommand: vi.fn(),
        NativeMemoryReadFileCommand: vi.fn(),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { RpcApi } from "@/app/store/rpc-api";
import { NativeMemoryHistoryModel, orderVersionsOldestFirst } from "./native-memory-history-model";

function meta(id: string): NativeMemoryVersionMeta {
    return {
        id,
        content_hash: "",
        parent_version_id: null,
        source: "human",
        source_detail: "{}",
        session_id: "",
        created_at: 0,
    };
}

describe("orderVersionsOldestFirst", () => {
    // versionsAtom() is newest-first: index 0 = newest, higher index = older.
    const versions = [meta("v3-newest"), meta("v2"), meta("v1-oldest")];

    it("returns [oldest, newest] regardless of click order — first click newer", () => {
        const [from, to] = orderVersionsOldestFirst("v3-newest", "v1-oldest", versions);
        expect(from).toBe("v1-oldest");
        expect(to).toBe("v3-newest");
    });

    it("returns [oldest, newest] regardless of click order — first click older", () => {
        // Regression for reagent P1: an earlier revision had this branch's
        // output backwards, putting the newer id in `from`.
        const [from, to] = orderVersionsOldestFirst("v1-oldest", "v3-newest", versions);
        expect(from).toBe("v1-oldest");
        expect(to).toBe("v3-newest");
    });

    it("handles two adjacent versions in either click order — v2 is older than v3-newest", () => {
        expect(orderVersionsOldestFirst("v2", "v3-newest", versions)).toEqual(["v2", "v3-newest"]);
        expect(orderVersionsOldestFirst("v3-newest", "v2", versions)).toEqual(["v2", "v3-newest"]);
    });

    it("falls back to input order when an id is not found", () => {
        expect(orderVersionsOldestFirst("unknown-a", "unknown-b", versions)).toEqual(["unknown-a", "unknown-b"]);
    });
});

function deferred<T>(): { promise: Promise<T>; resolve: (v: T) => void } {
    let resolve!: (v: T) => void;
    const promise = new Promise<T>((res) => {
        resolve = res;
    });
    return { promise, resolve };
}

// Regression for reagent P2 on PR #2678: computeDiff() had no request-id
// guard — selecting one version pair, then a different pair before the
// first NativeMemoryDiffCommand resolved, could let the stale response
// land after the newer one and overwrite diffTextAtom.
describe("NativeMemoryHistoryModel diff request staleness", () => {
    const versions = [meta("v3-newest"), meta("v2"), meta("v1-oldest")];
    let dispose: (() => void) | undefined;

    afterEach(() => {
        dispose?.();
        dispose = undefined;
        vi.clearAllMocks();
    });

    async function makeModel(): Promise<NativeMemoryHistoryModel> {
        vi.mocked(RpcApi.NativeMemoryHistoryCommand).mockResolvedValue({ versions });
        let model!: NativeMemoryHistoryModel;
        createRoot((d) => {
            dispose = d;
            model = new NativeMemoryHistoryModel("agent-1", "MEMORY.md");
        });
        // Let the constructor's fire-and-forget loadHistory() settle.
        await Promise.resolve();
        await Promise.resolve();
        return model;
    }

    it("discards a stale diff response that resolves after a newer selection's diff", async () => {
        const model = await makeModel();

        const first = deferred<NativeMemoryDiffResult>();
        const second = deferred<NativeMemoryDiffResult>();
        vi.mocked(RpcApi.NativeMemoryDiffCommand)
            .mockReturnValueOnce(first.promise)
            .mockReturnValueOnce(second.promise);

        // First pair selected — fires the first (slow) diff request.
        model.toggleDiffSelection("v1-oldest");
        model.toggleDiffSelection("v2");

        // Deselect one and pick a different pair before the first request
        // resolves — fires the second (fast) diff request.
        model.toggleDiffSelection("v2");
        model.toggleDiffSelection("v3-newest");

        // The second, newer request resolves first.
        second.resolve({ diff: "second diff" });
        await Promise.resolve();
        await Promise.resolve();
        expect(model.diffTextAtom()).toBe("second diff");

        // The first, now-stale request resolves after — must be discarded,
        // not overwrite the newer diff already shown.
        first.resolve({ diff: "first diff (stale)" });
        await Promise.resolve();
        await Promise.resolve();
        expect(model.diffTextAtom()).toBe("second diff");
    });

    it("discards a stale diff response after the selection is cleared entirely", async () => {
        const model = await makeModel();

        const pending = deferred<NativeMemoryDiffResult>();
        vi.mocked(RpcApi.NativeMemoryDiffCommand).mockReturnValueOnce(pending.promise);

        model.toggleDiffSelection("v1-oldest");
        model.toggleDiffSelection("v2");
        model.clearDiffSelection();

        pending.resolve({ diff: "stale diff" });
        await Promise.resolve();
        await Promise.resolve();
        expect(model.diffTextAtom()).toBeNull();
    });
});

describe("NativeMemoryHistoryModel current content", () => {
    const versions = [meta("v2-newest"), meta("v1-oldest")];
    let dispose: (() => void) | undefined;

    afterEach(() => {
        dispose?.();
        dispose = undefined;
        vi.clearAllMocks();
    });

    function makeModel(): NativeMemoryHistoryModel {
        vi.mocked(RpcApi.NativeMemoryHistoryCommand).mockResolvedValue({ versions });
        let model!: NativeMemoryHistoryModel;
        createRoot((d) => {
            dispose = d;
            model = new NativeMemoryHistoryModel("agent-1", "MEMORY.md");
        });
        return model;
    }

    it("starts with contentAtom null (loading) and fetches content on construction", async () => {
        vi.mocked(RpcApi.NativeMemoryReadFileCommand).mockResolvedValue({ content: "# hello" });
        const model = makeModel();
        expect(model.contentAtom()).toBeNull();

        await Promise.resolve();
        await Promise.resolve();
        expect(RpcApi.NativeMemoryReadFileCommand).toHaveBeenCalledWith(expect.anything(), {
            agent_id: "agent-1",
            filename: "MEMORY.md",
        });
        expect(model.contentAtom()).toBe("# hello");
    });

    it("distinguishes a genuinely empty file (empty string) from still-loading (null)", async () => {
        vi.mocked(RpcApi.NativeMemoryReadFileCommand).mockResolvedValue({ content: "" });
        const model = makeModel();
        expect(model.contentAtom()).toBeNull();

        await Promise.resolve();
        await Promise.resolve();
        expect(model.contentAtom()).toBe("");
        expect(model.contentAtom()).not.toBeNull();
    });

    it("records a content-fetch failure in contentErrorAtom, distinct from the history error", async () => {
        vi.mocked(RpcApi.NativeMemoryReadFileCommand).mockRejectedValue(new Error("disk gone"));
        const model = makeModel();

        await Promise.resolve();
        await Promise.resolve();
        await Promise.resolve();
        expect(model.contentAtom()).toBeNull();
        expect(model.contentErrorAtom()).toMatch(/disk gone/);
        expect(model.errorAtom()).toBeNull();
    });

    it("refreshes contentAtom with the reverted content after a successful revert", async () => {
        vi.mocked(RpcApi.NativeMemoryReadFileCommand)
            .mockResolvedValueOnce({ content: "old content" })
            .mockResolvedValueOnce({ content: "reverted content" });
        vi.mocked(RpcApi.NativeMemoryRevertCommand).mockResolvedValue({ version: meta("v3-revert") });

        const model = makeModel();
        await Promise.resolve();
        await Promise.resolve();
        expect(model.contentAtom()).toBe("old content");

        const reverted: string[] = [];
        model.onReverted = (content) => reverted.push(content);

        await model.revertTo("v1-oldest");

        expect(model.contentAtom()).toBe("reverted content");
        expect(reverted).toEqual(["reverted content"]);
    });

    // Regression for codex P2 on PR #3218: the constructor's initial
    // loadContent() and a revert-triggered loadContent() can both be in
    // flight at once; without a request-id guard the older response could
    // resolve second and clobber the newer one.
    it("discards a stale initial content response that resolves after a revert-triggered reload", async () => {
        const initial = deferred<NativeMemoryReadFileResult>();
        const afterRevert = deferred<NativeMemoryReadFileResult>();
        vi.mocked(RpcApi.NativeMemoryReadFileCommand)
            .mockReturnValueOnce(initial.promise)
            .mockReturnValueOnce(afterRevert.promise);
        vi.mocked(RpcApi.NativeMemoryRevertCommand).mockResolvedValue({ version: meta("v3-revert") });

        const model = makeModel();
        // Start the revert before the constructor's initial read resolves —
        // this fires the second (revert-triggered) loadContent() call while
        // the first is still pending.
        const revertPromise = model.revertTo("v1-oldest");
        await Promise.resolve();

        // The newer, revert-triggered read resolves first.
        afterRevert.resolve({ content: "reverted content" });
        await revertPromise;
        expect(model.contentAtom()).toBe("reverted content");

        // The stale initial read resolves after — must be discarded, not
        // overwrite the newer content already shown.
        initial.resolve({ content: "stale initial content" });
        await Promise.resolve();
        await Promise.resolve();
        expect(model.contentAtom()).toBe("reverted content");
    });

    // Regression for reagent P1 + codex P2 on PR #3218: forwarding
    // contentAtom() unconditionally after a revert could hand the caller a
    // stale pre-revert value when the post-revert content refresh itself
    // failed, silently presenting old content as freshly reverted.
    it("does not forward stale content via onReverted when the post-revert refresh fails", async () => {
        vi.mocked(RpcApi.NativeMemoryReadFileCommand)
            .mockResolvedValueOnce({ content: "old content" })
            .mockRejectedValueOnce(new Error("disk gone"));
        vi.mocked(RpcApi.NativeMemoryRevertCommand).mockResolvedValue({ version: meta("v3-revert") });

        const model = makeModel();
        await Promise.resolve();
        await Promise.resolve();
        expect(model.contentAtom()).toBe("old content");

        const reverted: string[] = [];
        model.onReverted = (content) => reverted.push(content);

        await model.revertTo("v1-oldest");

        expect(reverted).toEqual([]);
        expect(model.contentErrorAtom()).toMatch(/disk gone/);
        // Stale pre-revert value stays visible rather than being silently
        // treated as current — the error banner is what tells the user it's
        // out of date, not a wrong value passed off as fresh.
        expect(model.contentAtom()).toBe("old content");
    });

    it("shows contentLoadingAtom as true only while a content fetch is actually in flight", async () => {
        const pending = deferred<NativeMemoryReadFileResult>();
        vi.mocked(RpcApi.NativeMemoryReadFileCommand).mockReturnValueOnce(pending.promise);

        const model = makeModel();
        expect(model.contentLoadingAtom()).toBe(true);

        pending.resolve({ content: "hello" });
        await Promise.resolve();
        await Promise.resolve();
        expect(model.contentLoadingAtom()).toBe(false);
    });

    it("stops showing contentLoadingAtom after a content-fetch failure", async () => {
        vi.mocked(RpcApi.NativeMemoryReadFileCommand).mockRejectedValue(new Error("disk gone"));
        const model = makeModel();

        await Promise.resolve();
        await Promise.resolve();
        await Promise.resolve();
        expect(model.contentLoadingAtom()).toBe(false);
        expect(model.contentAtom()).toBeNull();
    });
});
