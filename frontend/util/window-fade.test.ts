// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { pageCarriesWindowFade } from "./window-fade";

describe("pageCarriesWindowFade", () => {
    it("is true only for an alpha-capable window, as the host says in the page's address", () => {
        expect(pageCarriesWindowFade("?ipc_port=1&window_transparent=1")).toBe(true);
        expect(pageCarriesWindowFade("?ipc_port=1&window_transparent=0")).toBe(false);
        expect(pageCarriesWindowFade("?ipc_port=1")).toBe(false);
        expect(pageCarriesWindowFade("")).toBe(false);
    });
});
