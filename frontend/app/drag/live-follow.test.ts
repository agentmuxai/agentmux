// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/util/logger", () => ({ Logger: { warn: vi.fn(), info: vi.fn(), error: vi.fn(), debug: vi.fn() } }));

import { setPlatform } from "@/util/platformutil";
import { beginLiveFollow, endLiveFollow, type LiveFollowSpec } from "./live-follow";

const spec: LiveFollowSpec = {
    sourceWindowLabel: "main",
    picture: "jpeg",
    windowWidth: 1200,
    windowHeight: 800,
    offsetX: 90,
    offsetY: 20,
    tearLineY: 42,
};

function fakeApi() {
    return { liveFollowBegin: vi.fn(async () => {}), liveFollowEnd: vi.fn(async () => {}) };
}

describe("live follow", () => {
    beforeEach(() => setPlatform("win32"));
    afterEach(() => setPlatform("darwin"));

    it("hands the host the picture and where it goes", async () => {
        const api = fakeApi();
        await beginLiveFollow(api, spec);
        expect(api.liveFollowBegin).toHaveBeenCalledWith(spec);
    });

    it("a refused picture is only lost: the failure is swallowed", async () => {
        const api = fakeApi();
        api.liveFollowBegin.mockRejectedValue(new Error("no window"));
        await expect(beginLiveFollow(api, spec)).resolves.toBeUndefined();
    });

    it("drops the picture when the real window is up", () => {
        const api = fakeApi();
        endLiveFollow(api);
        expect(api.liveFollowEnd).toHaveBeenCalledTimes(1);
    });

    it("does nothing off Windows", async () => {
        setPlatform("linux");
        const api = fakeApi();
        await beginLiveFollow(api, spec);
        endLiveFollow(api);
        expect(api.liveFollowBegin).not.toHaveBeenCalled();
        expect(api.liveFollowEnd).not.toHaveBeenCalled();
    });
});
