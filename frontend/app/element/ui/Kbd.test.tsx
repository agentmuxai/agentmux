// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";
import { Kbd } from "./Kbd";

afterEach(() => cleanup());

describe("Kbd", () => {
    it("is a key cap carrying the key's name, plus the caller's class", () => {
        const { container } = render(() => <Kbd class="mine">Enter</Kbd>);
        const k = container.querySelector("kbd.ui-kbd.mine");
        expect(k?.textContent).toBe("Enter");
    });
});
