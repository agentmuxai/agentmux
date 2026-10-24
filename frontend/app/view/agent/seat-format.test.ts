// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** How a seat reads on the My Agents tile and in the pane header's tooltip. */

import { describe, expect, it } from "vitest";

import type { SeatSummary } from "@/types/rpc/SeatSummary";
import { leadName, readSeatMeta, roleLabel, seatBadgeText, seatReportsLine, seatTooltipLine } from "./seat-format";

const leader: SeatSummary = { role: "leader", qualifier: "product", lead: null, area: "agent pane, identity, …" };
const worker: SeatSummary = {
    role: "worker",
    qualifier: null,
    lead: { id: "uid-camper", name: "Camper" },
    area: "panes, layout, tabs",
};

describe("seat-format", () => {
    it("labels the role, with the qualifier after it", () => {
        expect(seatBadgeText(leader)).toBe("Leader · product");
        expect(seatBadgeText(worker)).toBe("Worker");
        expect(roleLabel("platform_tester")).toBe("Platform tester");
        expect(roleLabel("captain")).toBe("captain");
    });

    it("names the lead: the operator, the agent, or one this machine can't name", () => {
        expect(leadName(null)).toBe("the operator");
        expect(leadName({ id: "x", name: "Camper" })).toBe("Camper");
        expect(leadName({ id: "x", name: null })).toBe("an agent on another machine");
    });

    it("writes the tile's line and the tooltip's line", () => {
        expect(seatReportsLine(worker)).toBe("reports to Camper · panes, layout, tabs");
        expect(seatReportsLine({ lead: null, area: null })).toBe("reports to the operator");
        expect(seatTooltipLine(leader)).toBe("Leader · product — reports to the operator — agent pane, identity, …");
        expect(seatTooltipLine({ ...worker, area: null })).toBe("Worker — reports to Camper");
    });

    it("reads agent:seat from block meta, and nothing it can't trust", () => {
        expect(readSeatMeta({ "agent:seat": worker })).toEqual(worker);
        expect(readSeatMeta({ "agent:seat": { role: "leader" } })).toEqual({
            role: "leader",
            qualifier: null,
            lead: null,
            area: null,
        });
        expect(readSeatMeta({})).toBeNull();
        expect(readSeatMeta(undefined)).toBeNull();
        expect(readSeatMeta({ "agent:seat": { role: "boss" } })).toBeNull();
        expect(readSeatMeta({ "agent:seat": "leader" })).toBeNull();
        expect(readSeatMeta({ "agent:seat": { role: "worker", lead: { id: "" } } })?.lead).toBeNull();
    });
});
