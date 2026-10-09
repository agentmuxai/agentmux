// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The build picks the host module (docs/specs/SPEC_EXTERNAL_HOST_BUILD_2026_10_09.md):
// the desktop host by default, or AGENTMUX_HOST_MODULE.

import * as path from "path";
import { afterEach, describe, expect, it, vi } from "vitest";

async function loadConfig(env: Record<string, string | undefined>) {
    const saved = { ...process.env };
    for (const [k, v] of Object.entries(env)) {
        // Assigning undefined would store the string "undefined".
        if (v === undefined) delete process.env[k];
        else process.env[k] = v;
    }
    try {
        vi.resetModules();
        return (await import("../../../vite.config")).default as { resolve?: { alias?: Record<string, string> } };
    } finally {
        process.env = saved;
    }
}

describe("the build's host module", () => {
    afterEach(() => vi.resetModules());

    it("is left to tsconfig.json's @host-module (the desktop host) when nothing is set", async () => {
        const config = await loadConfig({ AGENTMUX_HOST_MODULE: undefined, AGENTMUX_HOST_TSCONFIG: undefined });
        expect(config.resolve?.alias?.["@host-module"]).toBeUndefined();
    });

    it("is AGENTMUX_HOST_MODULE when set, as an absolute path", async () => {
        const config = await loadConfig({ AGENTMUX_HOST_MODULE: "some/dir/host.ts" });
        expect(config.resolve?.alias?.["@host-module"]).toBe(path.resolve("some/dir/host.ts"));
    });
});
