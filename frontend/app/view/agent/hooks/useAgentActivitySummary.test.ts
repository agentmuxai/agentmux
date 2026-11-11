// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useAgentActivitySummary — pins the session-goal-title trigger behavior
 * introduced by docs/specs/SPEC_AMBIENT_PANE_TITLE_OVERALL_GOAL_TRACKING_2026_08_17.md:
 * fires on entering `Submitting` (not turn completion), sends the literal
 * `pendingContent` as `user_message`, and only writes the result if it's
 * still the most recent request when it resolves.
 */

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { TurnPhase } from "@/app/store/agent-pane-state/types";
import type { DocumentNode } from "../../../agent-model/types";

const hub = vi.hoisted(() => ({
    activitySummary: vi.fn(),
    updateMeta: vi.fn(),
    /** The block's meta as the hook reads it back; updateMeta writes merge in. */
    meta: {} as Record<string, unknown>,
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        AgentActivitySummaryCommand: (...args: unknown[]) => hub.activitySummary(...args),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mos", () => ({ makeORef: (type: string, id: string) => `${type}:${id}` }));
vi.mock("@/app/store/services", () => ({
    ObjectService: { UpdateObjectMeta: (...args: unknown[]) => hub.updateMeta(...args) },
}));
vi.mock("@/app/store/token-usage", () => ({ recordTurn: vi.fn() }));
vi.mock("@/app/store/global", () => ({
    MOS: { getMuxObjectAtom: () => () => ({ meta: hub.meta }) },
}));

import { resetHumanTurns } from "@/app/store/title-schedule";
import { useAgentActivitySummary } from "./useAgentActivitySummary";

const BLOCK_ID = "b";

beforeEach(() => {
    hub.activitySummary.mockReset();
    hub.meta = {};
    resetHumanTurns(BLOCK_ID);
    hub.updateMeta.mockReset().mockImplementation((_oref: string, patch: Record<string, unknown>) => {
        for (const [k, v] of Object.entries(patch)) {
            if (v === null) delete hub.meta[k];
            else hub.meta[k] = v;
        }
        return Promise.resolve();
    });
});
afterEach(() => {
    vi.clearAllMocks();
});

function setup(initialPhase: TurnPhase = { kind: "Idle" }, document?: () => DocumentNode[]) {
    let setPhase!: (p: TurnPhase) => void;
    let dispose: () => void = () => {};
    createRoot((d) => {
        dispose = d;
        const [phase, setP] = createSignal<TurnPhase>(initialPhase);
        setPhase = setP;
        useAgentActivitySummary({
            blockId: BLOCK_ID,
            turnPhase: phase,
            getRootWidth: () => 400,
            document,
        });
    });
    return { setPhase, dispose };
}

describe("useAgentActivitySummary — trigger", () => {
    it("fires on entering Submitting, sending pendingContent as user_message", async () => {
        hub.activitySummary.mockResolvedValue({ summary: "new title", tokens: null });
        const { setPhase, dispose } = setup();

        setPhase({ kind: "Submitting", submittedAt: 1, pendingContent: "invert user input styling" });
        await Promise.resolve();
        await Promise.resolve();

        expect(hub.activitySummary).toHaveBeenCalledTimes(1);
        const [, payload] = hub.activitySummary.mock.calls[0];
        expect(payload.block_id).toBe(BLOCK_ID);
        expect(payload.user_message).toBe("invert user input styling");
        dispose();
    });

    it("never writes the title itself: the backend stores it (ambient/title.rs)", async () => {
        hub.activitySummary.mockResolvedValue({ summary: "new title", tokens: null });
        const { setPhase, dispose } = setup();
        setPhase({ kind: "Submitting", submittedAt: 1, pendingContent: "invert user input styling" });
        for (let i = 0; i < 4; i++) await Promise.resolve();
        const wrote = hub.updateMeta.mock.calls.some(([, patch]) => "term:ambient_summary" in patch);
        expect(wrote).toBe(false);
        dispose();
    });

    it("does not fire at mount even if the initial phase is already Submitting (defer: true)", async () => {
        const { dispose } = setup({ kind: "Submitting", submittedAt: 1, pendingContent: "already in flight" });
        await Promise.resolve();
        await Promise.resolve();

        expect(hub.activitySummary).not.toHaveBeenCalled();
        dispose();
    });

    it.each(["Idle", "Streaming", "Interrupting", "Done", "Disconnected"] as const)(
        "does not fire when the phase transitions to %s",
        async (kind) => {
            const { setPhase, dispose } = setup();
            const phase =
                kind === "Streaming"
                    ? ({ kind, bufferSize: 0, toolsActive: 0, lastEventMs: 0 } as TurnPhase)
                    : kind === "Interrupting"
                      ? ({ kind, reason: "user", sigintSentAt: 0 } as TurnPhase)
                      : kind === "Done"
                        ? ({ kind, outcome: "completed", finishedAt: 0 } as TurnPhase)
                        : kind === "Disconnected"
                          ? ({ kind, lastKind: "Streaming", lastConnectedAt: 0, reason: "stream-unsubscribed" } as TurnPhase)
                          : ({ kind } as TurnPhase);
            setPhase(phase);
            await Promise.resolve();

            expect(hub.activitySummary).not.toHaveBeenCalled();
            dispose();
        },
    );

    it("does NOT fire for a hidden turn (memory reinjection) — reagentx P0, PR #3502", async () => {
        // useAgentActivitySummary used to forward pendingContent
        // unconditionally, regardless of source — for a hidden memory-
        // reinjection turn (memory-reinjection-controller.ts), that meant
        // the full composed message got sent to an ambient LLM call whose
        // result becomes the human-visible pane title, defeating the whole
        // point of "hidden." A hidden turn's own pendingContent is now
        // always a content-free placeholder anyway (defense in depth), but
        // this test pins the actual gate: even if it somehow carried real
        // text, `hidden: true` alone must suppress the call outright.
        const { setPhase, dispose } = setup();

        setPhase({
            kind: "Submitting",
            submittedAt: 1,
            pendingContent: "some text that must never reach an ambient call",
            hidden: true,
        });
        await Promise.resolve();
        await Promise.resolve();

        expect(hub.activitySummary).not.toHaveBeenCalled();
        dispose();
    });

    it("ignores AgentMux's startup message: not a prompt, not a title's input (context delivery §9 C3)", async () => {
        const { setPhase, dispose } = setup();

        setPhase({ kind: "Submitting", submittedAt: 1, pendingContent: "# Session Context\n## Identity\n- **Name:** Cdtest" });
        await Promise.resolve();
        await Promise.resolve();

        expect(hub.activitySummary).not.toHaveBeenCalled();
        expect(hub.updateMeta).not.toHaveBeenCalled();
        expect(hub.meta["term:last_prompt"]).toBeUndefined();
        dispose();
    });

    it("still fires normally when hidden is explicitly false or omitted", async () => {
        hub.activitySummary.mockResolvedValue({ summary: "t", tokens: null });
        const { setPhase, dispose } = setup();

        setPhase({ kind: "Submitting", submittedAt: 1, pendingContent: "ordinary message", hidden: false });
        await Promise.resolve();
        await Promise.resolve();

        expect(hub.activitySummary).toHaveBeenCalledTimes(1);
        dispose();
    });
});

const submit = (setPhase: (p: TurnPhase) => void, n: number, text = `message ${n} about the login race`) => {
    setPhase({ kind: "Submitting", submittedAt: n, pendingContent: text });
    setPhase({ kind: "Streaming", bufferSize: 0, toolsActive: 0, lastEventMs: 0 });
};
/** The title was judged just now, at human turn `turn`: not aged, so only the schedule applies. */
const judgedNow = (turn: number) => {
    hub.meta["term:ambient_summary_at"] = Date.now();
    hub.meta["term:ambient_summary_turn"] = turn;
};
const flush = async () => {
    for (let i = 0; i < 4; i++) await Promise.resolve();
};

describe("useAgentActivitySummary — schedule (hardening PR 3)", () => {
    it("counts human turns in block meta, so a remount does not restart the count", async () => {
        hub.activitySummary.mockResolvedValue({ summary: "", tokens: null });
        const { setPhase, dispose } = setup();
        submit(setPhase, 1);
        submit(setPhase, 2);
        await flush();
        expect(hub.meta["term:human_turns"]).toBe(2);
        dispose();

        const again = setup();
        submit(again.setPhase, 3);
        await flush();
        expect(hub.meta["term:human_turns"]).toBe(3);
        again.dispose();
    });

    it("asks on every message while there is no title", async () => {
        hub.activitySummary.mockResolvedValue({ summary: "", tokens: null });
        const { setPhase, dispose } = setup();
        for (let n = 1; n <= 4; n++) submit(setPhase, n);
        await flush();
        expect(hub.activitySummary).toHaveBeenCalledTimes(4);
        dispose();
    });

    it("with a title, asks only on turns 2, 5 and 8", async () => {
        hub.meta["term:ambient_summary"] = "Fix the login race";
        judgedNow(1);
        hub.activitySummary.mockResolvedValue({ summary: "", tokens: null });
        const { setPhase, dispose } = setup();
        for (let n = 1; n <= 9; n++) submit(setPhase, n);
        await flush();
        const asked = hub.activitySummary.mock.calls.map(([, payload]) => payload.user_message);
        expect(asked).toEqual([2, 5, 8].map((n) => `message ${n} about the login race`));
        dispose();
    });

    it("counts messages sent before the meta write lands (muxreview P2 on #4238)", async () => {
        // The real write is a server round trip; until it lands the block meta
        // still shows the old count.
        hub.updateMeta.mockImplementation(() => new Promise(() => {}));
        hub.activitySummary.mockResolvedValue({ summary: "", tokens: null });
        hub.meta = { "term:ambient_summary": "Fix the login race" };
        judgedNow(0);
        const { setPhase, dispose } = setup();
        for (let n = 1; n <= 5; n++) submit(setPhase, n);
        await flush();
        const turns = hub.updateMeta.mock.calls.map(([, patch]) => patch["term:human_turns"]);
        expect(turns).toEqual([1, 2, 3, 4, 5]);
        // So the schedule held: turns 2 and 5 asked, the others did not.
        expect(hub.activitySummary).toHaveBeenCalledTimes(2);
        dispose();
    });

    it("a message on an unscheduled turn makes no request, so it can't supersede the scheduled one (#4238)", async () => {
        hub.activitySummary.mockImplementationOnce(() => new Promise(() => {}));
        hub.meta = { "term:ambient_summary": "Fix the login race", "term:human_turns": 4 };
        judgedNow(4);
        const { setPhase, dispose } = setup();
        submit(setPhase, 5); // scheduled: asks
        submit(setPhase, 6); // not scheduled: no request
        await flush();
        expect(hub.activitySummary).toHaveBeenCalledTimes(1);
        dispose();
    });
});


describe("useAgentActivitySummary — an old title does not outlive its goal", () => {
    const ask = (turns: number) => {
        hub.activitySummary.mockResolvedValue({ summary: "", tokens: null });
        const { setPhase, dispose } = setup();
        for (let n = 1; n <= turns; n++) submit(setPhase, n);
        return dispose;
    };
    const refreshes = () => hub.activitySummary.mock.calls.map(([, payload]) => [payload.turn, payload.refresh]);

    it("a title judged nine messages ago is judged afresh, with no current title shown", async () => {
        hub.meta["term:ambient_summary"] = "Compare AWS Bedrock and OpenRouter pricing";
        judgedNow(1);
        const dispose = ask(10);
        await flush();
        // 2, 5 and 8 are the schedule's (the model may answer SKIP); 10 is nine messages after turn 1.
        expect(refreshes()).toEqual([
            [2, false],
            [5, false],
            [8, false],
            [10, true],
        ]);
        dispose();
    });

    it("a title judged more than two hours ago is judged afresh at the next message", async () => {
        hub.meta["term:ambient_summary"] = "Compare AWS Bedrock and OpenRouter pricing";
        hub.meta["term:ambient_summary_at"] = Date.now() - 3 * 60 * 60_000;
        hub.meta["term:ambient_summary_turn"] = 0;
        const dispose = ask(1);
        await flush();
        expect(refreshes()).toEqual([[1, true]]);
        dispose();
    });

    it("a title with no stamp (an older build's, or restored) has no known age: judged afresh once", async () => {
        hub.meta["term:ambient_summary"] = "Compare AWS Bedrock and OpenRouter pricing";
        const dispose = ask(1);
        await flush();
        expect(refreshes()).toEqual([[1, true]]);
        dispose();
    });

    it("a compaction marks the title stale: the next message judges it afresh", async () => {
        hub.meta["term:ambient_summary"] = "Fix the login race";
        judgedNow(0);
        hub.meta["term:ambient_summary_at"] = 0; // markTitleStale
        const dispose = ask(1);
        await flush();
        expect(refreshes()).toEqual([[1, true]]);
        dispose();
    });

    it("a status question in the middle of new work carries that work with it", async () => {
        hub.meta["term:ambient_summary"] = "Compare AWS Bedrock and OpenRouter pricing";
        hub.meta["term:ambient_summary_at"] = Date.now() - 4 * 60 * 60_000;
        hub.meta["term:ambient_summary_turn"] = 1;
        hub.meta["term:human_turns"] = 20;
        hub.activitySummary.mockResolvedValue({ summary: "", tokens: null });
        const document = () =>
            [
                { type: "user_message", id: "u1", message: "now fix the Windows resize bug", isStartup: false },
                { type: "markdown", id: "m1", content: "Found it in the child-window lag.", metadata: {} },
            ] as unknown as DocumentNode[];
        const { setPhase, dispose } = setup({ kind: "Idle" }, document);
        submit(setPhase, 21, "whats the status on that?");
        await flush();
        const [, payload] = hub.activitySummary.mock.calls[0];
        expect(payload.user_message).toBe("whats the status on that?");
        expect(payload.activity).toEqual([
            "[user] now fix the Windows resize bug",
            "[assistant] Found it in the child-window lag.",
        ]);
        expect(payload.refresh).toBe(true);
        expect(payload.turn).toBe(21);
        dispose();
    });

    it("sends the turn the title is stamped with, and no activity from a pane without a document", async () => {
        const dispose = ask(1);
        await flush();
        const [, payload] = hub.activitySummary.mock.calls[0];
        expect(payload.turn).toBe(1);
        expect(payload.activity).toBeUndefined();
        dispose();
    });
});
