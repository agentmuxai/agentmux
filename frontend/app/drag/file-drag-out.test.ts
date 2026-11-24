// @vitest-environment jsdom
//
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";
import { makeTestHostApi } from "@/app/host/test-host";
import { FILE_DRAG_MARKER, markFileDragOut, prepareFileDragOut } from "./file-drag-out";

const flush = () => new Promise((r) => setTimeout(r, 0));
const dataTransfer = () => {
    const set = vi.fn();
    return { dt: { setData: set } as unknown as DataTransfer, set };
};

describe("file-drag-out: the nonce a drag from a row carries", () => {
    let register: ReturnType<typeof vi.fn>;
    beforeEach(() => {
        register = vi.fn(async () => "nonce-1");
        window.api = makeTestHostApi({ registerFileDrag: register as never });
    });

    it("registers the paths when the pointer goes down and marks the drag that follows with the nonce", async () => {
        prepareFileDragOut(["C:\\a\\x.png"]);
        await flush();
        expect(register).toHaveBeenCalledWith(["C:\\a\\x.png"]);
        const { dt, set } = dataTransfer();
        expect(markFileDragOut(dt, ["C:\\a\\x.png"])).toBe(true);
        expect(set).toHaveBeenCalledWith(FILE_DRAG_MARKER, "nonce-1");
    });

    it("marks a drag once, and only for the paths it registered", async () => {
        prepareFileDragOut(["a", "b"]);
        await flush();
        const other = dataTransfer();
        expect(markFileDragOut(other.dt, ["a"])).toBe(false);
        expect(other.set).not.toHaveBeenCalled();
        // The mismatch used the registration up: a later drag must register again.
        const again = dataTransfer();
        expect(markFileDragOut(again.dt, ["a", "b"])).toBe(false);
    });

    it("marks nothing when the host refused the files (a folder, a vault file), so the drag stays an in-app drag", async () => {
        register.mockRejectedValueOnce(new Error("not a file"));
        prepareFileDragOut(["C:\\a"]);
        await flush();
        const { dt, set } = dataTransfer();
        expect(markFileDragOut(dt, ["C:\\a"])).toBe(false);
        expect(set).not.toHaveBeenCalled();
    });

    it("marks nothing before the host has answered", () => {
        prepareFileDragOut(["C:\\a\\x.png"]);
        const { dt } = dataTransfer();
        expect(markFileDragOut(dt, ["C:\\a\\x.png"])).toBe(false);
    });

    it("on a host that cannot take files out, asks once and never again", async () => {
        register.mockResolvedValueOnce(null);
        prepareFileDragOut(["a"]);
        await flush();
        prepareFileDragOut(["b"]);
        await flush();
        expect(register).toHaveBeenCalledTimes(1);
    });
});
