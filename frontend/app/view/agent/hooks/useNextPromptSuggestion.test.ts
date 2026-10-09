// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useNextPromptSuggestion — pins the hidden-turn suppression added on
 * reagentx's second review round on PR #3502 (the "third leak" the first
 * review round flagged as plausible-but-unconfirmed, then confirmed on
 * re-review): NextPromptSuggestionCommand's backend implementation reads
 * the raw FileStore output tail directly, with no concept of "hidden" on
 * the server side — so a hidden memory-reinjection turn's full composed
 * <system-reminder> content, and the model's real reply to it, would
 * otherwise be sent to an ambient LLM call and the resulting suggestion
 * rendered as visible ghost text in the composer. The only available fix at
 * this layer is skipping the RPC call entirely for a hidden turn's own
 * completion — see useNextPromptSuggestion.ts's `lastTurnWasHidden` doc
 * comment for why passing `hidden` through the RPC payload wouldn't help.
 */

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { TurnPhase } from "@/app/store/agent-pane-state/types";
import type { DocumentNode } from "../types";

const hub = vi.hoisted(() => ({
    nextPromptSuggestion: vi.fn(),
    updateMeta: vi.fn(),
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        NextPromptSuggestionCommand: (...args: unknown[]) => hub.nextPromptSuggestion(...args),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mos", () => ({ makeORef: (type: string, id: string) => `${type}:${id}` }));
vi.mock("@/app/store/services", () => ({
    ObjectService: { UpdateObjectMeta: (...args: unknown[]) => hub.updateMeta(...args) },
}));
vi.mock("@/app/store/token-usage", () => ({ recordTurn: vi.fn() }));

import { shouldRequestSuggestion, useNextPromptSuggestion } from "./useNextPromptSuggestion";

const BLOCK_ID = "b";

beforeEach(() => {
    hub.nextPromptSuggestion.mockReset().mockResolvedValue({ suggestion: "next thing", tokens: null });
    hub.updateMeta.mockReset().mockResolvedValue(undefined);
});
afterEach(() => {
    vi.clearAllMocks();
});

function setup(isComposerEmpty: () => boolean = () => true, document?: () => DocumentNode[]) {
    let setPhase!: (p: TurnPhase) => void;
    let bumpTurnEnded!: () => void;
    let dispose: () => void = () => {};
    createRoot((d) => {
        dispose = d;
        const [phase, setP] = createSignal<TurnPhase>({ kind: "Idle" });
        const [turnEnded, setTurnEnded] = createSignal(0);
        setPhase = setP;
        bumpTurnEnded = () => setTurnEnded((n) => n + 1);
        useNextPromptSuggestion({
            blockId: BLOCK_ID,
            turnPhase: phase,
            turnJustEndedAtom: turnEnded,
            isComposerEmpty,
            document,
        });
    });
    return { setPhase, bumpTurnEnded, dispose };
}

describe("useNextPromptSuggestion — hidden-turn suppression (reagentx P0, PR #3502, round 2)", () => {
    it("does NOT call NextPromptSuggestionCommand when the just-ended turn was hidden", async () => {
        const { setPhase, bumpTurnEnded, dispose } = setup();

        setPhase({
            kind: "Submitting",
            submittedAt: 1,
            pendingContent: "[agentmux: hidden memory reinjection]",
            hidden: true,
        });
        await Promise.resolve();
        bumpTurnEnded(); // mirrors the real turn_active: true -> false edge
        await Promise.resolve();
        await Promise.resolve();

        expect(hub.nextPromptSuggestion).not.toHaveBeenCalled();
        dispose();
    });

    it("still calls NextPromptSuggestionCommand normally for an ordinary (non-hidden) turn", async () => {
        const { setPhase, bumpTurnEnded, dispose } = setup();

        setPhase({ kind: "Submitting", submittedAt: 1, pendingContent: "ordinary message", hidden: false });
        await Promise.resolve();
        setPhase({ kind: "Done", outcome: "completed", finishedAt: 2 });
        bumpTurnEnded();
        await Promise.resolve();
        await Promise.resolve();

        expect(hub.nextPromptSuggestion).toHaveBeenCalledTimes(1);
        expect(hub.updateMeta).toHaveBeenCalledWith(
            "block:b",
            expect.objectContaining({ "term:next_prompt_suggestion": "next thing" }),
        );
        dispose();
    });

    it("resumes firing normally on the NEXT (non-hidden) turn after a hidden one", async () => {
        const { setPhase, bumpTurnEnded, dispose } = setup();

        setPhase({ kind: "Submitting", submittedAt: 1, pendingContent: "hidden", hidden: true });
        await Promise.resolve();
        bumpTurnEnded();
        await Promise.resolve();
        await Promise.resolve();
        expect(hub.nextPromptSuggestion).not.toHaveBeenCalled();

        setPhase({ kind: "Submitting", submittedAt: 2, pendingContent: "a real message", hidden: false });
        await Promise.resolve();
        setPhase({ kind: "Done", outcome: "completed", finishedAt: 3 });
        bumpTurnEnded();
        await Promise.resolve();
        await Promise.resolve();

        expect(hub.nextPromptSuggestion).toHaveBeenCalledTimes(1);
        dispose();
    });
});

// A call that can only be thrown away, or has nothing to continue, isn't made.
// docs/reports/REPORT_AMBIENT_FRAMEWORK_REASSESSMENT_2026_10_08.md section 6.2.
describe("useNextPromptSuggestion — no call when there's nothing to suggest", () => {
    async function endTurn(setPhase: (p: TurnPhase) => void, bumpTurnEnded: () => void, end?: TurnPhase) {
        setPhase({ kind: "Submitting", submittedAt: 1, pendingContent: "go", hidden: false });
        await Promise.resolve();
        if (end) setPhase(end);
        bumpTurnEnded();
        await Promise.resolve();
        await Promise.resolve();
    }

    it("doesn't call while the user is already typing", async () => {
        const { setPhase, bumpTurnEnded, dispose } = setup(() => false);
        await endTurn(setPhase, bumpTurnEnded);
        expect(hub.nextPromptSuggestion).not.toHaveBeenCalled();
        dispose();
    });

    it("doesn't call after a turn that errored or was stopped", async () => {
        for (const outcome of ["errored", "stopped", "interrupted"] as const) {
            hub.nextPromptSuggestion.mockClear();
            const { setPhase, bumpTurnEnded, dispose } = setup();
            await endTurn(setPhase, bumpTurnEnded, { kind: "Done", outcome, finishedAt: 2 });
            expect(hub.nextPromptSuggestion, outcome).not.toHaveBeenCalled();
            dispose();
        }
    });

    it("doesn't call while the turn is being stopped", async () => {
        const { setPhase, bumpTurnEnded, dispose } = setup();
        await endTurn(setPhase, bumpTurnEnded, { kind: "Interrupting", reason: "user", sigintSentAt: 2 } as TurnPhase);
        expect(hub.nextPromptSuggestion).not.toHaveBeenCalled();
        dispose();
    });

    it("doesn't write a suggestion if the turn turns out stopped while the call ran", async () => {
        let resolve!: (v: unknown) => void;
        hub.nextPromptSuggestion.mockImplementationOnce(() => new Promise((r) => { resolve = r; }));
        const { setPhase, bumpTurnEnded, dispose } = setup();
        await endTurn(setPhase, bumpTurnEnded, { kind: "Done", outcome: "completed", finishedAt: 2 });
        expect(hub.nextPromptSuggestion).toHaveBeenCalledTimes(1);
        setPhase({ kind: "Done", outcome: "stopped", finishedAt: 3 });
        resolve({ suggestion: "next thing", tokens: null });
        await Promise.resolve();
        await Promise.resolve();
        const wrote = hub.updateMeta.mock.calls.some(([, patch]) => patch["term:next_prompt_suggestion"] === "next thing");
        expect(wrote).toBe(false);
        dispose();
    });

    it("waits for the turn's end to reach the pane before taking its activity (#4506)", async () => {
        const { setPhase, bumpTurnEnded, dispose } = setup();
        // The backend's edge arrives while the reply is still streaming in.
        await endTurn(setPhase, bumpTurnEnded, { kind: "Streaming", bufferSize: 0, toolsActive: 0, lastEventMs: 0 });
        expect(hub.nextPromptSuggestion).not.toHaveBeenCalled();
        setPhase({ kind: "Done", outcome: "completed", finishedAt: 2 });
        await new Promise((r) => setTimeout(r, 120));
        expect(hub.nextPromptSuggestion).toHaveBeenCalledTimes(1);
        dispose();
    });

    it("calls after a completed turn", async () => {
        const { setPhase, bumpTurnEnded, dispose } = setup();
        await endTurn(setPhase, bumpTurnEnded, { kind: "Done", outcome: "completed", finishedAt: 2 });
        expect(hub.nextPromptSuggestion).toHaveBeenCalledTimes(1);
        dispose();
    });

    it("sends the pane's recent activity with the request", async () => {
        const nodes = [
            { type: "user_message", id: "u", message: "fix it" },
            { type: "markdown", id: "m", content: "Fixed." },
        ] as unknown as DocumentNode[];
        const { setPhase, bumpTurnEnded, dispose } = setup(undefined, () => nodes);
        await endTurn(setPhase, bumpTurnEnded, { kind: "Done", outcome: "completed", finishedAt: 2 });
        expect(hub.nextPromptSuggestion.mock.calls[0][1]).toMatchObject({
            block_id: BLOCK_ID,
            activity: ["[user] fix it", "[assistant] Fixed."],
        });
        dispose();
    });

    it("the rule itself: an empty composer, and any phase but a failed or stopped end", () => {
        expect(shouldRequestSuggestion({ kind: "Idle" }, true)).toBe(true);
        expect(shouldRequestSuggestion({ kind: "Done", outcome: "completed", finishedAt: 1 }, true)).toBe(true);
        expect(shouldRequestSuggestion({ kind: "Done", outcome: "completed", finishedAt: 1 }, false)).toBe(false);
        expect(shouldRequestSuggestion({ kind: "Done", outcome: "errored", finishedAt: 1 }, true)).toBe(false);
        expect(shouldRequestSuggestion({ kind: "Interrupting", reason: "user", sigintSentAt: 1 } as TurnPhase, true)).toBe(false);
    });
});
