// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { startupWindowTransparent, transparencyNeedsRestart } from "./transparency-restart";

describe("startupWindowTransparent", () => {
    it("reads 1 and 0 from the host's window_transparent param", () => {
        expect(startupWindowTransparent("?ipc_port=1&window_transparent=1")).toBe(true);
        expect(startupWindowTransparent("?ipc_port=1&window_transparent=0")).toBe(false);
    });

    it("is unknown when the param is missing or malformed", () => {
        expect(startupWindowTransparent("?windowLabel=pool-1")).toBeNull();
        expect(startupWindowTransparent("?window_transparent=yes")).toBeNull();
    });
});

describe("transparencyNeedsRestart", () => {
    it("is true only on Linux, after turning transparency on in a session that started opaque", () => {
        expect(transparencyNeedsRestart({ linux: true, startup: false, transparent: true })).toBe(true);
    });

    it("is false when the session already started transparent", () => {
        expect(transparencyNeedsRestart({ linux: true, startup: true, transparent: true })).toBe(false);
    });

    it("is false when transparency is off", () => {
        expect(transparencyNeedsRestart({ linux: true, startup: false, transparent: false })).toBe(false);
    });

    it("is false off Linux, where transparency applies live", () => {
        expect(transparencyNeedsRestart({ linux: false, startup: false, transparent: true })).toBe(false);
    });

    it("is false when the startup value is unknown", () => {
        expect(transparencyNeedsRestart({ linux: true, startup: null, transparent: true })).toBe(false);
    });
});
