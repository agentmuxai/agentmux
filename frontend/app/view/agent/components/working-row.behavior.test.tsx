// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentWorkingRow, pinned as a whole before it was split into a state object, hook groups and view parts: each
 * scenario drives the row through a turn on a fake clock (a plain turn, srv's turn ledger with its lead-in,
 * settling and end, the held states, the cancellable login phases, compaction and its estimate bar,
 * reconnecting, a long-running tool, reduced motion) and logs, in order, the row as a tree and the number of
 * timers alive at each step. The logs are snapshots recorded on the unsplit component; the split must leave
 * them byte for byte.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createStore } from "solid-js/store";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const motion = vi.hoisted(() => ({ reduced: false }));
vi.mock("@/app/motion/motion", async (original) => ({
    ...(await original<typeof import("@/app/motion/motion")>()),
    shouldReduceMotion: () => motion.reduced,
}));

import { AgentWorkingRow } from "./working-row";

type Props = Parameters<typeof AgentWorkingRow>[0];

const T0 = 1_800_000_000_000;
const T: string[] = [];
const log = (line: string) => T.push(line);
const SNAP = (name: string) => `./__snapshots__/working-row.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const ATTRS = /^(class|role|aria-.*|data-.*|title|style|disabled|type)$/;

/** The row as an indented tree: tags, classes, the attributes that carry state, and text. */
function dump(node: Node, depth = 0, out: string[] = []): string[] {
    const pad = "  ".repeat(depth);
    if (node.nodeType === Node.TEXT_NODE) {
        const t = (node.textContent ?? "").trim();
        if (t) out.push(`${pad}"${t}"`);
        return out;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return out;
    const el = node as HTMLElement;
    const attrs = [...el.attributes].filter((a) => ATTRS.test(a.name)).map((a) => `${a.name}=${JSON.stringify(a.value)}`);
    out.push(`${pad}<${el.tagName.toLowerCase()}${attrs.length ? " " + attrs.join(" ") : ""}>`);
    el.childNodes.forEach((c) => dump(c, depth + 1, out));
    return out;
}

let container: HTMLElement;
let elapsed = 0;
let setProps: (p: Partial<Props>) => void;
const cancelLogin = vi.fn();

function mount(initial: Partial<Props> = {}) {
    const [p, set] = createStore<Props>({ loading: false, ...initial } as Props);
    setProps = (next) => set(next as never);
    container = render(() => (
        <AgentWorkingRow
            loading={p.loading}
            stopping={p.stopping}
            activitySummary={p.activitySummary}
            sessionStats={p.sessionStats}
            turnTokens={p.turnTokens}
            turnLedger={p.turnLedger}
            turnCarry={p.turnCarry}
            activity={p.activity}
            needsYou={p.needsYou}
            waitingReason={p.waitingReason}
            retryAtMs={p.retryAtMs}
            launchPhase={p.launchPhase}
            onCancelLogin={p.onCancelLogin}
            hasAuthUrl={p.hasAuthUrl}
            compacting={p.compacting}
            compactionContextTokens={p.compactionContextTokens}
            compactionModel={p.compactionModel}
            reconnecting={p.reconnecting}
        />
    )).container;
}

/** The row now, after `ms` more of the clock. */
function at(label: string, ms = 0) {
    if (ms) vi.advanceTimersByTime(ms);
    elapsed += ms;
    log(`== +${elapsed}ms ${label} (timers: ${vi.getTimerCount()})`);
    const row = container.firstElementChild;
    if (row) dump(row).forEach((l) => log(l));
    else log("-");
}

const now = () => Date.now();

const ledger = (over: Record<string, unknown> = {}) => ({
    turnId: 7,
    seq: 1,
    origin: null,
    trigger: { kind: "agent", from: "AgentY", external: true },
    absorbed: [],
    startedAtMs: now(),
    passes: 1,
    active: true,
    inputs: 0,
    countedPasses: 0,
    outputTokens: 0,
    costUsd: 0,
    steps: 0,
    durationApiMs: 0,
    lastPassEndedAtMs: null,
    settleUntilMs: null,
    endedAtMs: null,
    end: null,
    ...over,
}) as never;

beforeEach(() => {
    T.length = 0;
    elapsed = 0;
    motion.reduced = false;
    cancelLogin.mockReset();
    localStorage.clear();
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval", "setTimeout", "clearTimeout", "Date"] });
    vi.setSystemTime(T0);
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
});

describe("AgentWorkingRow behavior", () => {
    it("goes live, types its line out, counts, and ends in a worked line", async () => {
        log(`before mount (timers: ${vi.getTimerCount()})`);
        mount({ activitySummary: "Fix the login redirect loop" });
        at("idle, nothing to show");
        setProps({ loading: true });
        at("live, nothing typed");
        at("3 characters", 3 * 28 + 5);
        at("half", 300);
        at("whole", 2000);
        setProps({ turnTokens: { input: 5_000, output: 300, requesting: true } });
        at("requesting tokens", 1000);
        setProps({ turnTokens: { input: 5_000, output: 1_200 } });
        at("output counting", 1000);
        setProps({ turnTokens: { input: 5_000, output: 900 } });
        at("output never shown going down", 1000);
        at("30s: a new phrase", 30_000);
        setProps({ activitySummary: "Review the diff" });
        at("a new summary types out", 28 * 4);
        at("and finishes", 2000);
        setProps({ loading: false, sessionStats: { output_tokens: 2_400, duration_ms: 42_000, cost_usd: 0.1234, num_turns: 3 } });
        at("worked");
        setProps({ sessionStats: { duration_ms: 500 } });
        at("worked, short, no figures");
        setProps({ sessionStats: { output_tokens: 10 } });
        at("worked, no duration");
        setProps({ sessionStats: null });
        at("no stats: nothing");
        cleanup();
        log(`after unmount (timers: ${vi.getTimerCount()})`);
        await snapshot("turn");
    });

    it("follows srv's turn ledger: lead-in, settling between passes, and the whole turn's figures", async () => {
        mount({ activitySummary: "Answer the jekt" });
        setProps({ loading: true, turnLedger: ledger({ startedAtMs: now() - 2_000 }) });
        at("open turn, lead-in", 0);
        at("lead-in typing", 28 * 4);
        at("lead-in typed", 28 * 20);
        at("lead-in has passed", 6_000);
        setProps({ turnTokens: { input: 1_000, output: 400 } });
        at("tokens with the ledger", 1000);
        setProps({
            loading: false,
            turnLedger: ledger({ startedAtMs: now() - 9_000, active: false, passes: 1, settleUntilMs: now() + 3_000, lastPassEndedAtMs: now() }),
            sessionStats: { output_tokens: 400, duration_ms: 6_000, num_turns: 1 },
        });
        at("settling between passes: live, dimmed");
        at("a next pass starts", 1000);
        setProps({ loading: true, turnLedger: ledger({ startedAtMs: now() - 10_000, passes: 2 }) });
        at("second pass");
        setProps({
            loading: false,
            turnLedger: ledger({
                startedAtMs: now() - 12_000,
                active: false,
                passes: 2,
                countedPasses: 2,
                outputTokens: 5_300,
                costUsd: 0.456,
                steps: 9,
                endedAtMs: now(),
                end: "completed",
                absorbed: [
                    { kind: "user", from: null, external: false },
                    { kind: "user", from: null, external: false },
                    { kind: "agent", from: "Maop", external: true },
                ],
                inputs: 4,
            }),
        });
        at("ended: the whole turn's figures");
        setProps({ turnLedger: ledger({ startedAtMs: now() - 12_000, active: false, passes: 1, endedAtMs: now() - 1_000, trigger: { kind: "schedule", from: "cron", external: true } }) });
        at("ended, one pass, scheduled run");
        setProps({
            turnLedger: ledger({ turnId: 8, startedAtMs: now() - 12_000, active: false, passes: 2, countedPasses: 0, endedAtMs: now(), trigger: { kind: "task", from: null, external: true } }),
            turnCarry: { turnId: 8, passes: 2, outputTokens: 777, costUsd: 0.5, steps: 4 } as never,
        });
        at("ended, uncounted passes: the pane's own sum");
        setProps({ turnCarry: { turnId: 99, passes: 2, outputTokens: 1, costUsd: 1, steps: 1 } as never });
        at("another turn's carry is not used");
        setProps({ turnLedger: ledger({ startedAtMs: now() - 5_000, active: false, endedAtMs: null, lastPassEndedAtMs: now() - 1_000, settleUntilMs: null }) });
        at("ended by the clock, no end stamp", 1000);
        await snapshot("ledger");
    });

    it("says what holds the turn: needing you, a rate limit, stopping, a login phase", async () => {
        mount({ loading: true, activitySummary: "Ship it", onCancelLogin: cancelLogin });
        at("live", 3000);
        setProps({ needsYou: "Waiting for your approval: git push" });
        at("needs you", 3000);
        setProps({ needsYou: null, waitingReason: "rate_limited", retryAtMs: now() + 90_000 });
        at("rate limited, with a countdown", 3000);
        at("a second later", 1000);
        setProps({ retryAtMs: null });
        at("rate limited, no countdown", 3000);
        setProps({ waitingReason: null, stopping: true });
        at("stopping", 3000);
        setProps({ stopping: false, launchPhase: { kind: "waiting-for-login-link", deadlineMs: now() + 15_000 } });
        at("waiting for the login link: cancel offered", 3000);
        fireEvent.click(container.querySelector(".agent-working-row-cancel-login")!);
        log(`cancel clicks: ${cancelLogin.mock.calls.length}`);
        setProps({ hasAuthUrl: true });
        at("a URL was captured: no cancel", 3000);
        setProps({ hasAuthUrl: false, launchPhase: { kind: "opening-login-terminal" } });
        at("opening the login terminal", 3000);
        setProps({ launchPhase: { kind: "waiting-for-login-completion", deadlineMs: now() + 300_000 } });
        at("waiting for login completion", 3000);
        setProps({ launchPhase: { kind: "first-login" } });
        at("first login", 3000);
        setProps({ launchPhase: { kind: "auth-expired" } });
        at("auth expired", 3000);
        setProps({ launchPhase: { kind: "verifying" } });
        at("verifying: not cancellable", 3000);
        setProps({ launchPhase: { kind: "resolving-cli" }, onCancelLogin: undefined });
        at("no cancel handler", 3000);
        await snapshot("held");
    });

    it("estimates a compaction, from the default or from earlier ones, and gives way to reconnecting", async () => {
        mount({ loading: true, activitySummary: "Ship it" });
        setProps({ compacting: { trigger: "auto", startedAt: now() } });
        at("compacting, default estimate", 0);
        fireEvent.mouseEnter(container.querySelector(".agent-working-row-right")!);
        vi.advanceTimersByTime(1000);
        elapsed += 1000;
        log(`tooltip: ${JSON.stringify([...document.querySelectorAll("[role=tooltip], .tooltip, [class*=tooltip]")].map((e) => e.textContent))}`);
        at("20s in", 20_000);
        at("over the estimate", 120_000);
        setProps({ compacting: null });
        at("compaction over", 1000);
        localStorage.setItem(
            "agentmux:compaction-samples:v1",
            JSON.stringify([
                { uuid: "a", preTokens: 50_000, durationMs: 20_000, model: "m1" },
                { uuid: "b", preTokens: 100_000, durationMs: 40_000, model: "m1" },
                { uuid: "c", preTokens: 100_000, durationMs: 90_000, model: "m2" },
            ]),
        );
        setProps({ compactionContextTokens: 80_000, compactionModel: "m1", compacting: { trigger: "manual", startedAt: now() } });
        at("compacting, estimate from history", 0);
        fireEvent.mouseEnter(container.querySelector(".agent-working-row-right")!);
        vi.advanceTimersByTime(1000);
        elapsed += 1000;
        log(`tooltip: ${JSON.stringify([...document.querySelectorAll("[role=tooltip], .tooltip, [class*=tooltip]")].map((e) => e.textContent))}`);
        at("halfway", 15_000);
        setProps({ reconnecting: { startedAt: now() - 4_000 } as never });
        at("reconnecting takes over: no bar", 0);
        setProps({ compacting: null, loading: false });
        at("reconnecting alone, not loading", 5_000);
        setProps({ reconnecting: null });
        at("nothing", 1000);
        setProps({ compacting: { trigger: "auto", startedAt: now() }, compactionContextTokens: null, compactionModel: null });
        at("compacting while not loading", 2000);
        await snapshot("compaction");
    });

    it("names a long-running tool beside the spinner", async () => {
        mount({ loading: true, activitySummary: "Build it" });
        const tool = (id: string, startedAt: number, extra: Record<string, unknown> = {}) => ({
            id,
            activity: { family: "bash" as const, label: "Running the srv tests" },
            name: "Bash",
            command: "cargo test",
            startedAt,
            ...extra,
        });
        setProps({ activity: { phase: null, phaseSince: 0, tools: [tool("t1", now())], plan: null, planAt: 0 } as never });
        at("a tool just started", 500);
        at("a tool running a while", 30_000);
        at("a long while", 120_000);
        setProps({
            activity: { phase: null, phaseSince: 0, tools: [tool("t2", now() - 200_000, { parentId: "t1" })], plan: null, planAt: 0 } as never,
        });
        at("a subagent's call does not count", 1000);
        setProps({ needsYou: "Approve?" });
        at("needing you outranks it", 1000);
        await snapshot("operation");
    });

    it("jumps to the whole line when motion is reduced", async () => {
        motion.reduced = true;
        mount({ activitySummary: "Fix the login redirect loop" });
        setProps({ loading: true });
        at("live, whole at once");
        setProps({ needsYou: "Approve?" });
        at("a new line, whole at once");
        setProps({ needsYou: null, activitySummary: "" });
        at("no text", 100);
        await snapshot("reduced");
    });
});
