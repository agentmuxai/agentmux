// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { summarize } from "./summary";
import { endedLine, endedTip, goesAheadText, partTip, rowButton, rowButtonLabel, summaryTip } from "./summary-text";

describe("summary text", () => {
    it("spells each part's count out, in the singular for one", () => {
        expect(partTip("agents", 1)).toBe("1 agent is running");
        expect(partTip("agents", 20)).toBe("20 agents are running");
        expect(partTip("asks", 1)).toBe("1 agent asks you something");
        expect(partTip("browser", 2)).toBe("2 browser panes need you");
        expect(partTip("vault", 1)).toBe("1 Vault request waits on you");
        expect(partTip("tasks", 4)).toBe("4 Kanban tasks need you");
    });

    it("says the whole item in one sentence, and where a click goes", () => {
        const some = summarize({ running: 3, items: [], apps: [], uses: [], requests: [], tasks: 1 });
        expect(summaryTip(some)).toBe("3 agents are running, 1 Kanban task needs you. Click to see who");
        const idle = summarize({ running: 3, items: [], apps: [], uses: [], requests: [], tasks: 0 });
        expect(summaryTip(idle)).toBe("3 agents are running. Click to open the Swarm");
    });

    it("names a row's button by where it goes", () => {
        const pane = { to: "pane", item: { blockId: "b", name: "Korp", what: "", since: 0, local: true } } as never;
        expect(rowButton(pane, false)).toBe("Go");
        expect(rowButton(pane, true)).toBe("Open");
        expect(rowButtonLabel(pane, "Korp", true)).toBe("Open Korp");
        expect(rowButtonLabel(pane, "Korp", false)).toBe("Go to Korp");
        expect(rowButton({ to: "vault", id: "u" }, false)).toBe("Answer");
        expect(rowButtonLabel({ to: "vault", id: "u" }, "Clare", false)).toBe("Answer Clare");
        expect(rowButtonLabel({ to: "kanban" }, "Kanban", false)).toBe("Open the Kanban");
    });

    it("counts a countdown down to zero", () => {
        expect(goesAheadText(72_000, 0)).toBe("goes ahead in 1:12 unless you deny");
        expect(goesAheadText(5_000, 0)).toBe("goes ahead in 0:05 unless you deny");
        expect(goesAheadText(0, 9_000)).toBe("goes ahead in 0:00 unless you deny");
    });

    it("says how an ask that ended ended, and the asks that ended in the tooltip", () => {
        expect(endedLine({ how: "answered", since: 0, endedAt: 72_000 })).toBe("answered after 1:12");
        expect(endedLine({ how: "closed", since: 10_000, endedAt: 17_000 })).toBe("closed unanswered after 0:07");
        expect(endedTip(1)).toBe("1 ask ended in the last 2 minutes");
        expect(endedTip(3)).toBe("3 asks ended in the last 2 minutes");
        const gone = { key: "k", blockId: "b", name: "n", what: "w", since: 0, endedAt: 10_000, how: "answered" as const };
        const none = summarize({ running: 0, items: [], apps: [], uses: [], requests: [], tasks: 0, ended: [gone] });
        expect(summaryTip(none)).toBe("1 ask ended in the last 2 minutes. Click to see them");
        const some = summarize({ running: 2, items: [], apps: [], uses: [], requests: [], tasks: 0, ended: [gone] });
        expect(summaryTip(some)).toBe("2 agents are running, 1 ask ended in the last 2 minutes. Click to see them");
    });
});
