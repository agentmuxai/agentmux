// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A claimed card whose holder has been quiet says so (SPEC_LEADER_TOLD_WHEN_AN_AGENT_GOES_QUIET_2026_11_21.md §6):
// the same fact as the Swarm row, read from the holder srv names on the card.

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@atlaskit/pragmatic-drag-and-drop/element/adapter", () => ({ draggable: () => () => {} }));
vi.mock("./kanban-media", () => ({ CardMedia: () => <div /> }));
vi.mock("./kanban-colors", () => ({ agentColor: () => undefined }));
vi.mock("./kanban-seats", () => ({ useSeatIndex: () => () => ({ byUid: new Map(), uidByName: new Map() }), seatOf: () => null }));
vi.mock("@/app/agent-model/seat-format", () => ({ seatTooltipLine: () => "" }));
vi.mock("@/app/store/global", () => ({ MOS: { getMuxObjectAtom: () => () => undefined } }));
vi.mock("@/app/store/agent-names", async () => {
    const { createSignal } = await import("solid-js");
    const [agentRoster] = createSignal<{ id: string; slug: string; name: string }[]>([]);
    return { followAgentNames: () => {}, agentRoster };
});
vi.mock("@/app/store/srv-info", () => ({ srvInfo: () => ({ hostName: "narko" }) }));
vi.mock("@/app/store/float-modifier", () => ({ takeFloatRequest: () => false }));
vi.mock("@/app/element/link-open", () => ({ onLinkClick: () => {}, onLinkAuxClick: () => {} }));

import { KanbanCard } from "./kanban-card";
import type { KanbanAgent, KanbanTask } from "./kanban-types";

afterEach(cleanup);

const MIN = 60_000;
const T0 = 1_000_000;

function claimed(holder: Partial<KanbanAgent> | null, state: KanbanTask["state"] = "claimed"): KanbanTask {
    return {
        id: "w1",
        title: "Build it",
        payload: "",
        kind: "",
        target_agent: "",
        target_agent_uid: "",
        priority: 0,
        state,
        claimed_by: "ann",
        claimed_by_uid: "uid-ann",
        attempts: 1,
        max_attempts: 3,
        created_by: "operator",
        created_at: 1,
        updated_at: 1,
        holder: holder ? ({ name: "ann", block_id: "b1", state: "idle", ...holder } as KanbanAgent) : null,
        result: "",
        card: {
            num: 7,
            repo: "agentmuxai/agentmux",
            branch: "",
            pr_url: "",
            review_required: false,
            checklist: [],
            question: "",
            note: "",
            requested_by: "",
            requested_by_kind: "",
            request_text: "",
            filed_by: "",
            updated_at: 1,
        },
    } as KanbanTask;
}

const pillText = (container: HTMLElement) => [...container.querySelectorAll(".kanban-card__meta *")].map((e) => e.textContent ?? "").join("|");

describe("the quiet pill on a card", () => {
    it("says how long its holder has been quiet, counting from the card's own clock", () => {
        const { container } = render(() => (
            <KanbanCard task={claimed({ quiet_since_ms: T0 })} now={T0 + 22 * MIN + 5_000} agents={[]} onOpen={() => {}} />
        ));
        expect(pillText(container)).toContain("quiet 22 min");
    });

    it("is not drawn for a holder that is not quiet, a card with no holder, or a task that is not claimed", () => {
        const text = (t: KanbanTask) => pillText(render(() => <KanbanCard task={t} now={T0 + 22 * MIN} agents={[]} onOpen={() => {}} />).container);
        expect(text(claimed({}))).not.toContain("quiet");
        expect(text(claimed(null))).not.toContain("quiet");
        expect(text(claimed({ quiet_since_ms: T0 }, "review"))).not.toContain("quiet");
    });
});
