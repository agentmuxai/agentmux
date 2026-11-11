// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it } from "vitest";
import { DualProviderLogo, dualLogoTitle } from "./DualProviderLogo";

afterEach(() => cleanup());

describe("DualProviderLogo", () => {
    it("omits the vendor badge when vendor matches the harness (single-vendor provider, no override)", () => {
        const { container } = render(() => <DualProviderLogo harness="claude" vendor="claude" />);
        expect(container.querySelector(".dual-provider-logo-badge")).toBeNull();
    });

    it("shows the vendor badge when vendor differs from the harness", () => {
        const { container } = render(() => <DualProviderLogo harness="claude" vendor="anthropic" />);
        expect(container.querySelector(".dual-provider-logo-badge")).not.toBeNull();
    });

    it("shows a \"custom\" badge for an agent with a vendor base URL override", () => {
        const { container } = render(() => <DualProviderLogo harness="claude" vendor="custom" />);
        const badge = container.querySelector(".dual-provider-logo-badge");
        expect(badge).not.toBeNull();
        const el = container.querySelector(".dual-provider-logo") as HTMLElement;
        expect(el.dataset.hoverTip).toContain("custom endpoint");
    });

    it("omits the badge entirely when no vendor is passed", () => {
        const { container } = render(() => <DualProviderLogo harness="claude" />);
        expect(container.querySelector(".dual-provider-logo-badge")).toBeNull();
        const el = container.querySelector(".dual-provider-logo") as HTMLElement;
        expect(el.dataset.hoverTip).toBe("Claude Code harness");
    });

    it("renders a visually distinct icon for the Anthropic vendor badge than the Claude harness icon", () => {
        // Regression: ProviderLogo used to map both "claude" and "anthropic"
        // to the identical claude-color.svg asset, so a claude-harness agent
        // running on the anthropic vendor showed the same icon twice — the
        // badge existed but was visually redundant, defeating the point of
        // the harness-vs-vendor split this component exists to communicate.
        const { container } = render(() => <DualProviderLogo harness="claude" vendor="anthropic" />);
        const harnessIcon = container.querySelector(".dual-provider-logo > .provider-logo") as HTMLElement;
        const vendorIcon = container.querySelector(".dual-provider-logo-badge .provider-logo") as HTMLElement;
        expect(harnessIcon).not.toBeNull();
        expect(vendorIcon).not.toBeNull();
        expect(vendorIcon.innerHTML).not.toBe(harnessIcon.innerHTML);
    });

    it("names a route model in the tooltip when given a vendor label", () => {
        const { container } = render(() => (
            <DualProviderLogo harness="claude" vendor="deepseek" vendorLabel="DeepSeek V4.1 Flash via OpenRouter" />
        ));
        const el = container.querySelector(".dual-provider-logo") as HTMLElement;
        expect(el.dataset.hoverTip).toBe("Claude Code harness running on DeepSeek V4.1 Flash via OpenRouter");
    });

    it("draws no tooltip or name of its own when bare, so a button around it can name it", () => {
        const { container } = render(() => <DualProviderLogo harness="claude" vendor="deepseek" bare />);
        const el = container.querySelector(".dual-provider-logo") as HTMLElement;
        expect(el.dataset.hoverTip).toBeUndefined();
        expect(el.getAttribute("aria-label")).toBeNull();
        expect(container.querySelector(".dual-provider-logo-badge")).not.toBeNull();
    });

    it("dualLogoTitle gives the words the logo carries", () => {
        expect(dualLogoTitle("claude", "claude")).toBe("Claude Code harness");
        expect(dualLogoTitle("claude", "anthropic")).toBe("Claude Code harness running on Anthropic");
        expect(dualLogoTitle("claude", "deepseek", "DeepSeek V4 Pro via OpenRouter")).toBe(
            "Claude Code harness running on DeepSeek V4 Pro via OpenRouter"
        );
    });

    it("keeps the model in the title when no badge is drawn, a harness on a model of its own maker", () => {
        expect(dualLogoTitle("qwen", "qwen", "Qwen3.8 Max via OpenRouter")).toBe(
            "Qwen Code harness running on Qwen3.8 Max via OpenRouter"
        );
    });

    it("draws the makers of a route model with marks of their own", () => {
        for (const maker of ["deepseek", "qwen", "zhipu", "moonshot"]) {
            const { container } = render(() => <DualProviderLogo harness="claude" vendor={maker} />);
            const mark = container.querySelector(".dual-provider-logo-badge .provider-logo") as HTMLElement;
            expect(mark.querySelector("svg"), maker).not.toBeNull();
            // The letter mark every unknown provider gets is a <text> in the svg.
            expect(mark.querySelector("text"), maker).toBeNull();
            cleanup();
        }
    });

    it("draws a new badge when the model's vendor changes under a mounted logo", async () => {
        const [vendor, setVendor] = createSignal("deepseek");
        const { container } = render(() => <DualProviderLogo harness="claude" vendor={vendor()} size={14} bare />);
        const title = () => container.querySelector(".dual-provider-logo-badge svg title")?.textContent;
        expect(title()).toBe("DeepSeek");
        setVendor("moonshot");
        await Promise.resolve();
        expect(title()).toBe("MoonshotAI");
        setVendor("claude");
        await Promise.resolve();
        expect(container.querySelector(".dual-provider-logo-badge")).toBeNull();
    });
});

describe("DualProviderLogo's small badge", () => {
    it("rings a small badge in the app's border color, not the pane's background", () => {
        const { container } = render(() => <DualProviderLogo harness="qwen" vendor="zhipu" size={14} />);
        const badge = container.querySelector(".dual-provider-logo-badge") as HTMLElement;
        expect(badge.style.boxShadow).toBe("0 0 0 1px var(--border-color)");
    });

    it("keeps the softer shadow on a list row's larger badge", () => {
        const { container } = render(() => <DualProviderLogo harness="qwen" vendor="zhipu" size={40} />);
        const badge = container.querySelector(".dual-provider-logo-badge") as HTMLElement;
        expect(badge.style.boxShadow).not.toContain("--border-color");
    });
});
