// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/store/global", () => ({ getSettingsKeyAtom: () => () => undefined }));
vi.mock("@/util/app-api", () => ({ getApi: () => undefined }));
vi.mock("@/app/store/flash-notifications", () => ({ pushNotification: () => {} }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import {
    acceptStoreProgress,
    setStoreProgress,
    STORE_ASKING_WINDOW_LINE,
    STORE_INSTALLING_LINE,
    storeDownloadedLine,
    storeProgress,
    storeProgressLine,
    storeRestartAsk,
    storeStayedLine,
    storeTargetVersion,
    type StoreOffer,
} from "./app-update";

const MB = 1024 * 1024;
const downloading = (percent: number | null, done = 0, total = 0) => ({ phase: "downloading" as const, percent, doneBytes: done, totalBytes: total });

afterEach(() => setStoreProgress(null));

describe("the Store's progress line", () => {
    it("quotes the percent and the megabytes, as the Store reports them", () => {
        expect(storeProgressLine(downloading(42, 88 * MB, 210 * MB), "0.60.25")).toBe("Downloading 0.60.25: 42% (88 of 210 MB)");
    });

    it("leaves the size out when the Store gave none, and the version out when nobody knows it", () => {
        expect(storeProgressLine(downloading(7), "0.60.25")).toBe("Downloading 0.60.25: 7%");
        expect(storeProgressLine(downloading(7, 1 * MB, 20 * MB), "")).toBe("Downloading the update: 7% (1 of 20 MB)");
    });

    it("says installing once the Store is deploying", () => {
        expect(storeProgressLine({ phase: "installing", percent: null, doneBytes: 0, totalBytes: 0 }, "0.60.25")).toBe(STORE_INSTALLING_LINE);
    });

    it("says the Store is asking, and where to look, for as long as it is", () => {
        const asking = { phase: "asking" as const, percent: null, doneBytes: 0, totalBytes: 0 };
        expect(storeProgressLine(asking, "")).toBe("The Microsoft Store is asking you to confirm: look for its window.");
        expect(storeProgressLine(asking, "")).toBe(STORE_ASKING_WINDOW_LINE);
    });

    it("gives no line, so the caller keeps its plain one, when there is no report or no numbers", () => {
        expect(storeProgressLine(null, "0.60.25")).toBeNull();
        expect(storeProgressLine(downloading(null), "0.60.25")).toBeNull();
        expect(storeProgressLine({ phase: "pending", percent: null, doneBytes: 0, totalBytes: 0 }, "")).toBeNull();
    });
});

describe("taking a report from the host", () => {
    it("keeps a well-formed report and clamps the percent", () => {
        acceptStoreProgress({ phase: "downloading", percent: 140, doneBytes: 5, totalBytes: 9 });
        expect(storeProgress()).toEqual({ phase: "downloading", percent: 100, doneBytes: 5, totalBytes: 9 });
        acceptStoreProgress({ phase: "installing", percent: null });
        expect(storeProgress()).toEqual({ phase: "installing", percent: null, doneBytes: 0, totalBytes: 0 });
    });

    it("keeps saying the Store is asking when its own earlier states arrive after the question", () => {
        acceptStoreProgress({ phase: "asking", percent: null });
        acceptStoreProgress({ phase: "pending", percent: null });
        acceptStoreProgress({ phase: "downloading", percent: 100, doneBytes: 9, totalBytes: 9 });
        expect(storeProgress()?.phase).toBe("asking");
        // Once the person answers, the Store deploys: that replaces the question.
        acceptStoreProgress({ phase: "installing", percent: null });
        expect(storeProgress()?.phase).toBe("installing");
    });

    it("ignores anything malformed", () => {
        acceptStoreProgress({ phase: "downloading", percent: 10, doneBytes: 1, totalBytes: 2 });
        for (const bad of [null, undefined, "x", {}, { phase: "flying" }, { phase: 3 }]) acceptStoreProgress(bad);
        expect(storeProgress()?.percent).toBe(10);
    });
});

describe("the version a Store update will be", () => {
    const offer = (version: string): StoreOffer => ({ state: "available", version, message: "", checkedMs: 1 });

    it("is the Store's own when it named a newer one", () => {
        expect(storeTargetVersion(offer("0.60.25"))).toBe("0.60.25");
    });

    it("is empty when the Store named none (the 2026-11-12 run left the marker's target empty): GitHub's latest is not guessed", () => {
        expect(storeTargetVersion(offer(""))).toBe("");
        expect(storeTargetVersion(null)).toBe("");
    });
});

describe("the downloaded row and the restart question", () => {
    it("promises a reopen only for Restart now, never for closing AgentMux by hand", () => {
        const line = storeDownloadedLine("0.60.25");
        expect(line).toBe("Downloaded 0.60.25. Restart now installs it: AgentMux closes and opens again. Closing AgentMux yourself does not install it.");
        expect(storeDownloadedLine("")).toContain("Downloaded. Restart now installs it");
    });

    it("names the agents in the middle of a turn that a restart interrupts", () => {
        expect(storeRestartAsk(["Vmer"], 3)).toBe(
            "Restart now closes AgentMux, stops its 3 agents, installs the update and opens AgentMux again. Vmer is in the middle of a turn and will be interrupted.",
        );
        expect(storeRestartAsk(["Vmer", "Deke"], 5)).toContain("Vmer and Deke are in the middle of a turn");
        expect(storeRestartAsk(["A", "B", "C", "D", "E"], 5)).toContain("A, B, C and 2 more are in the middle of a turn");
    });

    it("still says what a restart closes when nobody is named", () => {
        expect(storeRestartAsk([], 0)).toBe("Restart now closes AgentMux, stops every agent in it, installs the update and opens AgentMux again.");
        expect(storeRestartAsk([], 2)).not.toContain("middle of a turn");
    });

    it("says at the next start that the reopen could not be arranged", () => {
        const line = storeStayedLine({ state: "stayed", from: "0.60.24", target: "", reopen_failed: true });
        expect(line).toContain("AgentMux could not arrange to open itself after the update");
        expect(storeStayedLine({ state: "stayed", from: "0.60.24", target: "" })).not.toContain("could not arrange");
    });
});
