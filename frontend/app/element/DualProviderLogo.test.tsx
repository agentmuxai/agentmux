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
});
