// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";
import { DualProviderLogo } from "./DualProviderLogo";

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
        expect(el.title).toContain("custom endpoint");
    });

    it("omits the badge entirely when no vendor is passed", () => {
        const { container } = render(() => <DualProviderLogo harness="claude" />);
        expect(container.querySelector(".dual-provider-logo-badge")).toBeNull();
        const el = container.querySelector(".dual-provider-logo") as HTMLElement;
        expect(el.title).toBe("Claude Code harness");
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
});
