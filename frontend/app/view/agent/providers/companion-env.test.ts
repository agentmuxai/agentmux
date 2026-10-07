// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { companionCliEnv } from "./companion-env";

describe("companionCliEnv", () => {
    it("points pi-acp at the pi installed beside it", () => {
        expect(companionCliEnv("pi", String.raw`C:\cli\pi\0.0.34\node_modules\.bin\pi-acp.cmd`)).toEqual({
            PI_ACP_PI_COMMAND: String.raw`C:\cli\pi\0.0.34\node_modules\.bin\pi.cmd`,
        });
        expect(companionCliEnv("pi", "/home/u/cli/pi/0.0.34/node_modules/.bin/pi-acp")).toEqual({
            PI_ACP_PI_COMMAND: "/home/u/cli/pi/0.0.34/node_modules/.bin/pi",
        });
    });

    it("gives other providers nothing", () => {
        expect(companionCliEnv("claude", String.raw`C:\x\.bin\claude.cmd`)).toEqual({});
        expect(companionCliEnv("pi", "pi-acp")).toEqual({});
    });
});
