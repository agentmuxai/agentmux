// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";

import { buildAgentPaneIcon } from "./AgentPaneIcon";

afterEach(() => cleanup());

const face = (decl: IconButtonDecl) => render(() => (decl.icon as unknown as () => Element)() as never);

describe("buildAgentPaneIcon", () => {
    it("is the harness with the model's maker as the badge, named in the title", () => {
        const decl = buildAgentPaneIcon({
            harness: "claude",
            vendor: "deepseek",
            modelText: "DeepSeek V4.1 Flash via OpenRouter",
        });
        expect(decl.title).toBe("Claude Code harness running on DeepSeek V4.1 Flash via OpenRouter");
        expect(decl.noAction).toBe(true);
        const { container } = face(decl);
        expect(container.querySelector(".dual-provider-logo-badge")).not.toBeNull();
    });

    it("leaves the logo's own tooltip to the button, so the header shows one", () => {
        const { container } = face(buildAgentPaneIcon({ harness: "claude", vendor: "deepseek" }));
        const logo = container.querySelector(".dual-provider-logo") as HTMLElement;
        expect(logo.dataset.hoverTip).toBeUndefined();
    });

    it("has no badge when the harness answers itself", () => {
        const decl = buildAgentPaneIcon({ harness: "codex", vendor: "codex" });
        expect(decl.title).toBe("Codex CLI harness");
        expect(face(decl).container.querySelector(".dual-provider-logo-badge")).toBeNull();
    });

    it("gives each place the header draws it its own face", () => {
        const decl = buildAgentPaneIcon({ harness: "claude", vendor: "anthropic" });
        const first = face(decl).container.querySelector(".dual-provider-logo");
        const second = face(decl).container.querySelector(".dual-provider-logo");
        expect(first).not.toBeNull();
        expect(second).not.toBeNull();
        expect(first).not.toBe(second);
    });
});
