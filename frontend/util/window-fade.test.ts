// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { pageCarriesWindowFade } from "./window-fade";

describe("pageCarriesWindowFade", () => {
    it("is true only when the host says the page carries the window opacity", () => {
        expect(pageCarriesWindowFade({ __AGENTMUX_PAGE_FADE__: true })).toBe(true);
        expect(pageCarriesWindowFade({ __AGENTMUX_PAGE_FADE__: false })).toBe(false);
        expect(pageCarriesWindowFade({})).toBe(false);
    });
});
