// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The drop indicator's three looks, and that it follows state changes (the old overlay didn't). */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it } from "vitest";
import { DropIndicator } from "./DropIndicator";
import type { PaneDropState } from "./file-drop";

afterEach(() => cleanup());

describe("DropIndicator", () => {
    it("draws armed, target and blocked, and updates when the state changes", () => {
        const [state, setState] = createSignal<PaneDropState>({ state: "armed" });
        const { container } = render(() => <DropIndicator state={state()} />);
        const root = () => container.querySelector(".drop-indicator")!;
        expect(root().classList.contains("drop-indicator--armed")).toBe(true);
        expect(container.querySelector(".drop-indicator__prompt")).toBeNull();

        setState({ state: "target", message: "Drop 2 files to attach", icon: "fa-paperclip" });
        expect(root().classList.contains("drop-indicator--target")).toBe(true);
        expect(container.querySelector(".drop-indicator__prompt")?.textContent).toBe("Drop 2 files to attach");
        expect(container.querySelector(".fa-paperclip")).not.toBeNull();

        setState({ state: "blocked", reason: "No working folder for this agent" });
        const prompt = container.querySelector(".drop-indicator__prompt")!;
        expect(prompt.classList.contains("drop-indicator__prompt--blocked")).toBe(true);
        expect(prompt.textContent).toBe("No working folder for this agent");
        expect(root().getAttribute("aria-hidden")).toBe("true");
    });
});
