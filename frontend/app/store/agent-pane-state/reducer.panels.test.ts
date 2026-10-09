// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The pane reducer: panels, attached tasks and failure recovery.

import { describe, expect, it } from "vitest";
import { update } from "./reducer";
import { AgentPaneState, isWorking, TurnPhase, isAuthFailure, type PaneFailure } from "./types";
import { streaming, mk, ready } from "./reducer-test-fixtures";

describe("agent-pane-state reducer", () => {
    // ── Composer details / Log panel ────────────────────────────────
    describe("Composer details panel", () => {
        it("initial state: detailsOpen false", () => {
            const s = mk();
            expect(s.detailsOpen).toBe(false);
        });

        it("DetailsToggle: closed → open", () => {
            let s = mk();
            const r = update(s, { type: "DetailsToggle" });
            expect(r.state.detailsOpen).toBe(true);
            expect(r.events).toEqual([]);
        });

        it("DetailsToggle: open → closed", () => {
            let s = mk();
            s = update(s, { type: "DetailsToggle" }).state;
            s = update(s, { type: "DetailsToggle" }).state;
            expect(s.detailsOpen).toBe(false);
        });

        it("DetailsExpand: idempotent when already open", () => {
            let s = mk();
            s = update(s, { type: "DetailsExpand" }).state;
            const r = update(s, { type: "DetailsExpand" });
            expect(r.state).toBe(s);
        });

        it("DetailsExpand: opens when closed", () => {
            const s = mk();
            const r = update(s, { type: "DetailsExpand" });
            expect(r.state.detailsOpen).toBe(true);
        });

        it("DetailsCollapse: idempotent when already closed", () => {
            const s = mk();
            const r = update(s, { type: "DetailsCollapse" });
            expect(r.state).toBe(s);
        });

        it("DetailsCollapse: closes an open panel", () => {
            let s = mk();
            s = update(s, { type: "DetailsExpand" }).state;
            const r = update(s, { type: "DetailsCollapse" });
            expect(r.state.detailsOpen).toBe(false);
        });

        it("TurnStart leaves an open details panel open", () => {
            // The panel now hosts a live interactive shell (AgentShellSubblock)
            // that must survive sending more messages, so TurnStart no longer
            // force-closes it — the auto-collapse-on-send behavior was removed.
            let s = ready(100);
            s = update(s, { type: "DetailsExpand" }).state;
            expect(s.detailsOpen).toBe(true);
            const r = update(s, { type: "TurnStart", at: 200 });
            expect(r.state.detailsOpen).toBe(true);
            expect(r.state.turnPhase.kind).toBe("Submitting");
        });
    });

    // ── Stash drawer ────────────────────────────────────────────────────
    // SPEC_AGENT_STASH_PANE_MIGRATION_2026_09_22.md §3.3. Deliberately the
    // same command triple as the composer details panel above (the shell
    // drawer) rather than a bespoke atom: the Stash drawer is the same
    // category of per-pane UI state, and `endIconButtons()`'s toggle-highlight
    // memo (agent-model.ts) needs a reactive read, which the pane view's
    // per-field signals already provide for reducer state.
    describe("Stash drawer", () => {
        it("initial state: stashOpen false", () => {
            const s = mk();
            expect(s.stashOpen).toBe(false);
        });

        it("StashToggle: closed → open", () => {
            const s = mk();
            const r = update(s, { type: "StashToggle" });
            expect(r.state.stashOpen).toBe(true);
            expect(r.events).toEqual([]);
        });

        it("StashToggle: open → closed", () => {
            let s = mk();
            s = update(s, { type: "StashToggle" }).state;
            s = update(s, { type: "StashToggle" }).state;
            expect(s.stashOpen).toBe(false);
        });

        it("StashExpand: idempotent when already open", () => {
            let s = mk();
            s = update(s, { type: "StashExpand" }).state;
            const r = update(s, { type: "StashExpand" });
            expect(r.state).toBe(s);
        });

        it("StashExpand: opens when closed", () => {
            const s = mk();
            const r = update(s, { type: "StashExpand" });
            expect(r.state.stashOpen).toBe(true);
        });

        it("StashCollapse: idempotent when already closed", () => {
            const s = mk();
            const r = update(s, { type: "StashCollapse" });
            expect(r.state).toBe(s);
        });

        it("StashCollapse: closes an open drawer", () => {
            let s = mk();
            s = update(s, { type: "StashExpand" }).state;
            const r = update(s, { type: "StashCollapse" });
            expect(r.state.stashOpen).toBe(false);
        });

        it("is independent of the shell drawer — both can be open at once", () => {
            // §6 open question 1, answered: mutual exclusion would silently
            // close a live shell (which owns real terminal state), so the two
            // drawers are independent axes. The transcript is protected by a
            // CSS max-height on the Stash drawer instead (§3.1), not by
            // forcing one closed here.
            let s = mk();
            s = update(s, { type: "DetailsExpand" }).state;
            s = update(s, { type: "StashExpand" }).state;
            expect(s.detailsOpen).toBe(true);
            expect(s.stashOpen).toBe(true);

            s = update(s, { type: "StashCollapse" }).state;
            expect(s.detailsOpen).toBe(true); // shell untouched
            expect(s.stashOpen).toBe(false);
        });

        it("TurnStart leaves an open Stash drawer open", () => {
            // Same reasoning as the details panel above: sending a message
            // must not yank a surface the user deliberately opened.
            let s = ready(100);
            s = update(s, { type: "StashExpand" }).state;
            const r = update(s, { type: "TurnStart", at: 200 });
            expect(r.state.stashOpen).toBe(true);
            expect(r.state.turnPhase.kind).toBe("Submitting");
        });
    });

    // SPEC_ATTACHED_TASK_STATUS_AXIS_2026_08_02.md — a sibling axis to
    // TurnPhase for "is there a live agent-declared long-running task
    // attached to this pane," independent of turn state.
    describe("Attached task axis (AttachedTaskObserved / AttachedTaskCleared)", () => {
        it("initial state: attachedTask null", () => {
            const s = mk();
            expect(s.attachedTask).toBeNull();
        });

        it("AttachedTaskObserved sets attachedTask with the given timestamp", () => {
            const s = mk();
            const r = update(s, { type: "AttachedTaskObserved", at: 100 });
            expect(r.state.attachedTask).toEqual({ since: 100 });
            expect(r.events).toEqual([{ type: "attached-task-observed", at: 100 }]);
        });

        it("AttachedTaskObserved is idempotent — a second task starting doesn't reset `since`", () => {
            let s = mk();
            s = update(s, { type: "AttachedTaskObserved", at: 100 }).state;
            const r = update(s, { type: "AttachedTaskObserved", at: 200 });
            expect(r.state).toBe(s); // same-ref no-op
            expect(r.state.attachedTask).toEqual({ since: 100 });
            expect(r.events).toEqual([]);
        });

        it("AttachedTaskCleared clears an active episode", () => {
            let s = mk();
            s = update(s, { type: "AttachedTaskObserved", at: 100 }).state;
            const r = update(s, { type: "AttachedTaskCleared" });
            expect(r.state.attachedTask).toBeNull();
            expect(r.events).toEqual([{ type: "attached-task-cleared" }]);
        });

        it("AttachedTaskCleared is idempotent when already null", () => {
            const s = mk();
            const r = update(s, { type: "AttachedTaskCleared" });
            expect(r.state).toBe(s);
            expect(r.events).toEqual([]);
        });

        it("survives TurnEnd/TurnReset — a task attached in one turn can outlive it", () => {
            let s = ready(100);
            s = update(s, { type: "AttachedTaskObserved", at: 150 }).state;
            s = update(s, { type: "TurnStart", at: 200 }).state;
            s = update(s, { type: "TurnEnd", stats: null }).state;
            expect(s.attachedTask).toEqual({ since: 150 });
            s = update(s, { type: "TurnReset" }).state;
            expect(s.attachedTask).toEqual({ since: 150 });
        });

        it("survives ReconcileTurnActive / FailureObserved — orthogonal to every turn-lifecycle arm", () => {
            let s = ready(100);
            s = update(s, { type: "AttachedTaskObserved", at: 150 }).state;
            s = update(s, { type: "ReconcileTurnActive", at: 200, active: true }).state;
            expect(s.attachedTask).toEqual({ since: 150 });
            s = update(s, {
                type: "FailureObserved",
                failure: { code: "rate_limited", title: "Rate limited" } as AgentFailure,
                at: 300,
            }).state;
            expect(s.attachedTask).toEqual({ since: 150 });
        });
    });

    // Phase C of SPEC_BACKGROUND_TASK_DASHBOARD_INTELLIGENCE_2026_08_20.md
    // — a SEPARATE axis from attachedTask above, exclusively owned by
    // useBackgroundTaskRegistry.ts (see registryAttachedTaskSince's doc
    // comment in types.ts for why: agent-view.tsx's own attached-task
    // effect independently recomputes and clears `attachedTask` from the
    // transcript alone, which would otherwise stomp a direct dispatch into
    // that same field for a task the transcript has no record of).
    describe("Registry-derived attached-task floor (RegistryAttachedTaskObserved / RegistryAttachedTaskCleared)", () => {
        it("initial state: registryAttachedTaskSince null", () => {
            const s = mk();
            expect(s.registryAttachedTaskSince).toBeNull();
        });

        it("RegistryAttachedTaskObserved sets registryAttachedTaskSince with the given timestamp", () => {
            const s = mk();
            const r = update(s, { type: "RegistryAttachedTaskObserved", at: 100 });
            expect(r.state.registryAttachedTaskSince).toBe(100);
            expect(r.events).toEqual([{ type: "registry-attached-task-observed", at: 100 }]);
        });

        it("RegistryAttachedTaskObserved is a no-op (same ref) when the value is unchanged", () => {
            let s = mk();
            s = update(s, { type: "RegistryAttachedTaskObserved", at: 100 }).state;
            const r = update(s, { type: "RegistryAttachedTaskObserved", at: 100 });
            expect(r.state).toBe(s);
            expect(r.events).toEqual([]);
        });

        it("RegistryAttachedTaskObserved DOES update when a fresher/different value arrives — unlike AttachedTaskObserved's edge-trigger no-op", () => {
            let s = mk();
            s = update(s, { type: "RegistryAttachedTaskObserved", at: 100 }).state;
            const r = update(s, { type: "RegistryAttachedTaskObserved", at: 50 });
            expect(r.state.registryAttachedTaskSince).toBe(50);
        });

        it("RegistryAttachedTaskCleared clears an active value", () => {
            let s = mk();
            s = update(s, { type: "RegistryAttachedTaskObserved", at: 100 }).state;
            const r = update(s, { type: "RegistryAttachedTaskCleared" });
            expect(r.state.registryAttachedTaskSince).toBeNull();
            expect(r.events).toEqual([{ type: "registry-attached-task-cleared" }]);
        });

        it("RegistryAttachedTaskCleared is idempotent when already null", () => {
            const s = mk();
            const r = update(s, { type: "RegistryAttachedTaskCleared" });
            expect(r.state).toBe(s);
            expect(r.events).toEqual([]);
        });

        it("is fully independent of attachedTask — neither axis's commands touch the other's field", () => {
            let s = mk();
            s = update(s, { type: "AttachedTaskObserved", at: 100 }).state;
            s = update(s, { type: "RegistryAttachedTaskObserved", at: 200 }).state;
            expect(s.attachedTask).toEqual({ since: 100 });
            expect(s.registryAttachedTaskSince).toBe(200);
            s = update(s, { type: "AttachedTaskCleared" }).state;
            expect(s.attachedTask).toBeNull();
            expect(s.registryAttachedTaskSince).toBe(200); // untouched by AttachedTaskCleared
            s = update(s, { type: "RegistryAttachedTaskCleared" }).state;
            expect(s.registryAttachedTaskSince).toBeNull();
        });

        it("survives TurnEnd/TurnReset, same as attachedTask", () => {
            let s = ready(100);
            s = update(s, { type: "RegistryAttachedTaskObserved", at: 150 }).state;
            s = update(s, { type: "TurnStart", at: 200 }).state;
            s = update(s, { type: "TurnEnd", stats: null }).state;
            expect(s.registryAttachedTaskSince).toBe(150);
            s = update(s, { type: "TurnReset" }).state;
            expect(s.registryAttachedTaskSince).toBe(150);
        });
    });

    // docs/status/STATUS_STALE_RESUME_LIVE_REPRO_AND_FIX_PLAN_2026_08_23.md
    // §6.2 — mirrors the Attached task axis above (same shape, same
    // idempotent edge-trigger semantics), but for the backend's
    // stale-`--resume` retry/recovery signal instead.
    describe("Stale-resume reconnect axis (ResumeRetryStarted / ResumeRetryResolved)", () => {
        it("initial state: reconnecting null", () => {
            const s = mk();
            expect(s.reconnecting).toBeNull();
        });

        it("ResumeRetryStarted sets reconnecting with the given timestamp", () => {
            const s = mk();
            const r = update(s, { type: "ResumeRetryStarted", at: 100 });
            expect(r.state.reconnecting).toEqual({ startedAt: 100 });
            expect(r.events).toEqual([{ type: "resume-retry-started", at: 100 }]);
        });

        it("ResumeRetryStarted is idempotent — a cascaded retry doesn't reset startedAt", () => {
            let s = mk();
            s = update(s, { type: "ResumeRetryStarted", at: 100 }).state;
            const r = update(s, { type: "ResumeRetryStarted", at: 200 });
            expect(r.state).toBe(s); // same-ref no-op
            expect(r.state.reconnecting).toEqual({ startedAt: 100 });
            expect(r.events).toEqual([]);
        });

        it("ResumeRetryResolved clears an in-progress reconnect", () => {
            let s = mk();
            s = update(s, { type: "ResumeRetryStarted", at: 100 }).state;
            const r = update(s, { type: "ResumeRetryResolved" });
            expect(r.state.reconnecting).toBeNull();
            expect(r.events).toEqual([{ type: "resume-retry-resolved" }]);
        });

        it("ResumeRetryResolved is idempotent when already null", () => {
            const s = mk();
            const r = update(s, { type: "ResumeRetryResolved" });
            expect(r.state).toBe(s);
            expect(r.events).toEqual([]);
        });
    });

    // SPEC_AGENT_PANE_UNIFIED_FAILURE_REDUCER_2026_07_06.md — folds
    // useAgentFailure's local failure state into the reducer so a backend
    // failure classification unconditionally ends a working turn, instead
    // of depending on the CLI process actually exiting (which never
    // happens between turns for persistent-mode agents). Fixes the
    // "stuck Waiting after a rate-limit interruption" bug.
    describe("Failure recovery (FailureObserved / FailureCleared)", () => {
        const rateLimited: AgentFailure = {
            code: "rate_limited",
            title: "Rate limited",
            detail: "429",
            retryable: true,
        };

        it("FailureObserved while Streaming ends the turn (Done.errored) and records state.failure", () => {
            const s0 = streaming(100);
            const r = update(s0, { type: "FailureObserved", failure: rateLimited, at: 200 });
            expect(r.state.turnPhase).toEqual({ kind: "Done", outcome: "errored", finishedAt: 200 });
            // turnAttempted defaults to true — a backend-classified failure always
            // follows a turn that ran (PLAN_LOGIN_CTA_SURFACE_CONSOLIDATION_2026_09_02).
            expect(r.state.failure).toEqual({ data: rateLimited, at: 200, turnAttempted: true });
            expect(r.events).toContainEqual({ type: "failure-observed", code: "rate_limited", turnWasEnded: true });
        });

        it("FailureObserved while Submitting ends the turn (Done.errored)", () => {
            const s0 = update(ready(100), { type: "TurnStart", at: 100 }).state;
            expect(s0.turnPhase.kind).toBe("Submitting");
            const r = update(s0, { type: "FailureObserved", failure: rateLimited, at: 150 });
            expect(r.state.turnPhase).toEqual({ kind: "Done", outcome: "errored", finishedAt: 150 });
        });

        it("FailureObserved while Interrupting ends the turn (Done.errored)", () => {
            const s0 = update(streaming(100), { type: "RequestStop", at: 150 }).state;
            expect(s0.turnPhase.kind).toBe("Interrupting");
            const r = update(s0, { type: "FailureObserved", failure: rateLimited, at: 160 });
            expect(r.state.turnPhase).toEqual({ kind: "Done", outcome: "errored", finishedAt: 160 });
        });

        it("FailureObserved while Idle leaves the phase untouched but still records state.failure (turnWasEnded: false)", () => {
            const s0 = ready(100);
            expect(s0.turnPhase.kind).toBe("Idle");
            const r = update(s0, { type: "FailureObserved", failure: rateLimited, at: 150 });
            expect(r.state.turnPhase.kind).toBe("Idle");
            expect(r.state.failure).toEqual({ data: rateLimited, at: 150, turnAttempted: true });
            expect(r.events).toContainEqual({ type: "failure-observed", code: "rate_limited", turnWasEnded: false });
        });

        it("FailureObserved that ends a turn clears currentTool/currentToolArg/turnTokens", () => {
            let s0 = streaming(100);
            s0 = update(s0, { type: "ToolStart", name: "Bash", arg: "ls" }, 110).state;
            s0 = update(s0, { type: "TokensIn", input: 500 }, 110).state;
            expect(s0.currentTool).toBe("Bash");
            const r = update(s0, { type: "FailureObserved", failure: rateLimited, at: 200 });
            expect(r.state.currentTool).toBeNull();
            expect(r.state.currentToolArg).toBeNull();
            expect(r.state.turnTokens).toBeNull();
        });

        // PLAN_LOGIN_CTA_SURFACE_CONSOLIDATION_2026_09_02: the pre-launch
        // "never signed in" case (which used to render its own separate blue
        // "Log in" bar) is raised as a synthetic auth failure carrying
        // turnAttempted:false. That flag selects relogin()'s retryAfterLogin,
        // so getting it wrong re-sends the agent's last OLD message.
        it("FailureObserved records turnAttempted:false when the command sets it", () => {
            const s0 = ready(100);
            const r = update(s0, {
                type: "FailureObserved",
                failure: { code: "auth", title: "Not signed in", detail: "", retryable: true },
                at: 200,
                turnAttempted: false,
            });
            expect(r.state.failure?.turnAttempted).toBe(false);
        });

        it("FailureObserved defaults turnAttempted to true when the command omits it", () => {
            const s0 = streaming(100);
            const r = update(s0, { type: "FailureObserved", failure: rateLimited, at: 200 });
            expect(r.state.failure?.turnAttempted).toBe(true);
        });

        it("FailureCleared clears a synthetic pre-launch failure the same as any other", () => {
            const s0 = ready(100);
            const s1 = update(s0, {
                type: "FailureObserved",
                failure: { code: "auth", title: "Not signed in", detail: "", retryable: true },
                at: 200,
                turnAttempted: false,
            }).state;
            expect(s1.failure).not.toBeNull();
            expect(update(s1, { type: "FailureCleared" }).state.failure).toBeNull();
        });

        it("FailureCleared clears state.failure", () => {
            const s0 = update(streaming(100), { type: "FailureObserved", failure: rateLimited, at: 150 }).state;
            expect(s0.failure).not.toBeNull();
            const r = update(s0, { type: "FailureCleared" });
            expect(r.state.failure).toBeNull();
            expect(r.events).toEqual([{ type: "failure-cleared" }]);
        });

        it("FailureCleared with no active failure is a same-ref no-op", () => {
            const s0 = ready(100);
            const r = update(s0, { type: "FailureCleared" });
            expect(r.state).toBe(s0);
            expect(r.events).toEqual([]);
        });

        it("TurnStart implicitly clears a pre-existing state.failure (fresh turn ends the episode)", () => {
            let s0 = ready(100);
            s0 = update(s0, { type: "FailureObserved", failure: rateLimited, at: 150 }).state;
            expect(s0.failure).not.toBeNull();
            const r = update(s0, { type: "TurnStart", at: 200 });
            expect(r.state.failure).toBeNull();
            expect(r.state.turnPhase.kind).toBe("Submitting");
            expect(r.events).toContainEqual({ type: "failure-cleared" });
        });

        it("TurnStart with no active failure does NOT emit failure-cleared", () => {
            const s0 = ready(100);
            const r = update(s0, { type: "TurnStart", at: 200 });
            expect(r.events.some((e) => e.type === "failure-cleared")).toBe(false);
        });
    });

    // Issue 2 of ANALYSIS_AGENT_INPUT_LIFECYCLE_RATELIMIT_SENDNOW_2026_07_06.md:
    // StreamFlushObserved's Streaming arm previously spread the whole prior
    // phase forward unchanged, so a stale `waitingReason`/`retryAfterMs` from
    // an earlier rate-limit rode along through any later plain-text flush —
    // the reported false-positive "Rate limited — retrying…" label shown
    // long after the agent resumed normal streaming.
    describe("StreamFlushObserved clears stale rate-limit fields (false-positive fix)", () => {
        it("clears waitingReason/retryAfterMs on the next flush after a rate-limit wait", () => {
            let s0 = streaming(100);
            s0 = update(
                s0,
                { type: "ProviderWaiting", reason: "rate_limited", retryAfterMs: 5000, at: 110 },
            ).state;
            expect(s0.turnPhase).toMatchObject({ waitingReason: "rate_limited", retryAfterMs: 5000 });

            const r = update(s0, { type: "StreamFlushObserved", addedCount: 1, at: 120 });
            expect(r.state.turnPhase.kind).toBe("Streaming");
            expect((r.state.turnPhase as Extract<TurnPhase, { kind: "Streaming" }>).waitingReason).toBeUndefined();
            expect((r.state.turnPhase as Extract<TurnPhase, { kind: "Streaming" }>).retryAfterMs).toBeUndefined();
        });
    });
});


// `workingByLegacy` was the dual-write invariant helper (turnActive ||
// stopping) — removed in PR G alongside the legacy fields it read.
// Use `isWorking(state)` directly.

// Whether the model has ended its turn (`stop_reason: end_turn`) while the
// turn is still open. The busy predicate's background-work carve-out applies
// only then; mid-turn, the model is working even with no tool call running
// (docs/retro/retro-agent-pane-progress-flicker-and-orphaned-background-tasks-2026-09-30.md).
describe("ModelEndedTurn / ModelMessageStarted", () => {
    const flag = (s: AgentPaneState) =>
        s.turnPhase.kind === "Streaming" ? s.turnPhase.modelEndedTurn : "not streaming";

    it("a new Streaming turn starts with the model still working", () => {
        expect(flag(streaming())).toBeFalsy();
    });

    it("ModelEndedTurn marks the streaming turn's model as done", () => {
        const r = update(streaming(), { type: "ModelEndedTurn" });
        expect(flag(r.state)).toBe(true);
        expect(r.events).toEqual([]);
    });

    it("keeps the rest of the Streaming phase", () => {
        const s0 = streaming(100);
        const r = update(s0, { type: "ModelEndedTurn" });
        expect(r.state.turnPhase).toEqual({ ...s0.turnPhase, modelEndedTurn: true });
    });

    it("ModelMessageStarted clears it: the model is working again", () => {
        const s1 = update(streaming(), { type: "ModelEndedTurn" }).state;
        expect(flag(update(s1, { type: "ModelMessageStarted" }).state)).toBe(false);
    });

    it("is a same-ref no-op when nothing changes", () => {
        const s0 = streaming();
        expect(update(s0, { type: "ModelMessageStarted" }).state).toBe(s0);
        const s1 = update(s0, { type: "ModelEndedTurn" }).state;
        expect(update(s1, { type: "ModelEndedTurn" }).state).toBe(s1);
    });

    it("does nothing outside a Streaming turn", () => {
        const s0 = ready();
        expect(update(s0, { type: "ModelEndedTurn" }).state).toBe(s0);
        expect(update(s0, { type: "ModelMessageStarted" }).state).toBe(s0);
    });

    it("the next turn starts fresh", () => {
        const ended = update(streaming(100), { type: "ModelEndedTurn" }).state;
        const done = update(ended, { type: "TurnEnd", stats: null }).state;
        const next = update(update(done, { type: "TurnStart", at: 200 }).state, {
            type: "StreamFlushObserved",
            addedCount: 1,
            at: 200,
        }).state;
        expect(flag(next)).toBeFalsy();
    });
});

describe("isAuthFailure", () => {
    const f = (code: string) => ({ data: { code } }) as unknown as Pick<PaneFailure, "data">;
    it("is true only for an auth failure", () => {
        expect(isAuthFailure(f("auth"))).toBe(true);
        expect(isAuthFailure(f("rate_limited"))).toBe(false);
        expect(isAuthFailure(null)).toBe(false);
        expect(isAuthFailure(undefined)).toBe(false);
    });
});
