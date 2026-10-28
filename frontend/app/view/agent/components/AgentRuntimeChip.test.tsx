// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it } from "vitest";
import { AgentRuntimeChip, runtimeChip } from "./AgentRuntimeChip";

afterEach(cleanup);

describe("AgentRuntimeChip", () => {
    it("says each state in words", () => {
        expect(runtimeChip("waiting_on_user").label).toBe("waiting on you");
        expect(runtimeChip("signed_out").label).toBe("signed out");
        expect(runtimeChip("working").label).toBe("working");
        expect(runtimeChip("stopped").tone).toBe("neutral");
        expect(runtimeChip("error").tone).toBe("error");
    });

    it("says a reason code in words, and passes one it does not know through", () => {
        const { container } = render(() => <AgentRuntimeChip state="idle" detail="starts_on_first_message" />);
        expect(container.querySelector(".agent-runtime-chip")!.getAttribute("title")).toBe("Runs on its first message");
        cleanup();
        const other = render(() => <AgentRuntimeChip state="limited" detail="something_new" />);
        expect(other.container.querySelector(".agent-runtime-chip")!.getAttribute("title")).toBe("something_new");
    });

    it("follows the agent from one state to the next, and carries the reason as its tooltip", () => {
        const [state, setState] = createSignal<"working" | "limited">("working");
        const { container } = render(() => <AgentRuntimeChip state={state()} detail="usage_limit" />);
        const chip = () => container.querySelector(".agent-runtime-chip")!;
        expect(chip().textContent).toBe("working");
        expect(chip().getAttribute("title")).toBe("Out of usage until it resets");
        setState("limited");
        expect(chip().textContent).toBe("limited");
        expect(chip().className).toContain("agent-runtime-chip--limited");
    });
});
