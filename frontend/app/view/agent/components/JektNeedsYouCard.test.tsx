// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * How a notice's signature state is worded: the Needs-you card, a jekt's delivery tag and its meta row. A signature
 * that was genuine and failed only the age rules reads "signature expired (arrived late)" and is muted; an invalid
 * or missing signature still reads as an unproven sender, in the warning tone.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { JektMessageNode } from "../../../agent-model/types";
import { JektNeedsYouCard } from "./JektNeedsYouCard";
import { JektMeta, JektTags } from "./JektParts";
import { sigLabel, sigTone, trustLine, trustState } from "./jekt-trust-wording";

afterEach(() => cleanup());

function jekt(over: Partial<JektMessageNode> = {}): JektMessageNode {
    return {
        type: "jekt_message",
        id: "jm-1",
        from: "github-consumer",
        to: "manoz",
        message: "PR reviewed",
        raw: "[JEKT:...][/JEKT]",
        tier: "sensitive",
        deliveryTier: "wan",
        trust: "network-claimed",
        msgId: "inj-1",
        priority: "normal",
        direction: "incoming",
        timestamp: 1,
        escalate: "required",
        ...over,
    };
}

function card(over: Partial<JektMessageNode> = {}) {
    const { container } = render(() => (
        <JektNeedsYouCard jekt={jekt(over)} earlier={0} onShow={vi.fn()} onDismiss={vi.fn()} close={vi.fn()} />
    ));
    return container.querySelector(".jekt-needs-you-trust") as HTMLElement;
}

describe("the Needs-you card's trust line", () => {
    it("says a late but genuine signature expired and arrived late, muted", () => {
        const line = card({ sig: "expired" });
        expect(line.textContent).toBe("Signature expired (arrived late), trust: network-claimed");
        expect(line.getAttribute("data-trust")).toBe("expired");
    });

    it("keeps saying the sender is not proven for an invalid signature", () => {
        const line = card({ sig: "invalid" });
        expect(line.textContent).toBe("Sender not proven (network-claimed)");
        expect(line.getAttribute("data-trust")).toBe("unproven");
    });

    it("keeps saying the sender is not proven for a notice with no signature", () => {
        const line = card({});
        expect(line.textContent).toBe("Sender not proven (network-claimed)");
        expect(line.getAttribute("data-trust")).toBe("unproven");
    });

    it("keeps saying the sender is proven for a verified one", () => {
        const line = card({ trust: "wan-verified", sig: "verified" });
        expect(line.textContent).toBe("Sender proven (wan-verified)");
        expect(line.getAttribute("data-trust")).toBe("proven");
    });
});

describe("the wording of each signature state", () => {
    it("is one function for the card, the tags and the origin row", () => {
        expect(trustState("network-claimed", "expired")).toBe("expired");
        expect(trustState("network-claimed", "invalid")).toBe("unproven");
        expect(trustState("network-claimed", undefined)).toBe("unproven");
        expect(trustState("lan-verified", undefined)).toBe("proven");
        expect(trustLine("lan-verified", "verified")).toBe("Sender proven (lan-verified)");
        expect([sigLabel("verified"), sigLabel("expired"), sigLabel("invalid")]).toEqual([
            "signed",
            "signature expired (arrived late)",
            "signature invalid",
        ]);
        expect([sigTone("verified"), sigTone("expired"), sigTone("invalid")]).toEqual(["neutral", "neutral", "warn"]);
    });
});

describe("a jekt's delivery tag and meta row", () => {
    it("mutes the delivery tag for an expired signature", () => {
        const { container } = render(() => <JektTags node={jekt({ sig: "expired" })} />);
        const delivery = [...container.querySelectorAll(".ui-tag")].at(-1) as HTMLElement;
        expect(delivery.classList.contains("ui-tag--warn")).toBe(false);
    });

    it("keeps the warning tone on the delivery tag when no signature proved the sender", () => {
        for (const sig of [undefined, "invalid"] as const) {
            const { container } = render(() => <JektTags node={jekt({ sig })} />);
            const delivery = [...container.querySelectorAll(".ui-tag")].at(-1) as HTMLElement;
            expect(delivery.classList.contains("ui-tag--warn")).toBe(true);
            cleanup();
        }
    });

    it("adds a signature line to the meta row for an expired one only", () => {
        const expired = render(() => <JektMeta node={jekt({ sig: "expired" })} />);
        expect(expired.container.textContent).toContain("Signature: signature expired (arrived late)");
        cleanup();
        for (const sig of [undefined, "invalid", "verified"] as const) {
            const other = render(() => <JektMeta node={jekt({ sig })} />);
            expect(other.container.textContent).not.toContain("Signature:");
            cleanup();
        }
    });
});
