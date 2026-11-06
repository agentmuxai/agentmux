// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentQuestionPanel, pinned as a whole before it was split into hook groups: each scenario drives the panel
 * (answers, the recommended defaults, the auto-timeout, Cancel, activity, a published timer) and logs, in
 * order, what the DOM shows, what reaches `onAnswer` / `onCancel` and which question-timer calls were made.
 * The logs are snapshots recorded on the unsplit component; the split must leave them byte for byte.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import * as realTimer from "@/app/store/question-timer";
import type { ToolNode } from "../../../agent-model/types";
import { AgentQuestionPanel, recommendedOptions } from "./AgentQuestionPanel";

const T: string[] = [];
const log = (line: string) => T.push(line);

// The timer store, with every call logged. A key that is not a real block runs the real timer; a block's
// key (`blk-…`) is only logged, so nothing is written to block meta.
const isBlock = (key: string) => key.startsWith("blk");
const expiries = new Map<string, () => void>();
vi.mock("@/app/store/question-timer", async (importOriginal) => {
    const real = await importOriginal<typeof import("@/app/store/question-timer")>();
    return {
        ...real,
        startQuestionTimer: (key: string, opts: { durationMs: number; onExpire: () => void; publish?: boolean }) => {
            log(`startQuestionTimer(${isBlock(key) ? key : "local"}, ${opts.durationMs}ms, publish=${opts.publish ?? false})`);
            if (isBlock(key)) expiries.set(key, opts.onExpire);
            else real.startQuestionTimer(key, opts);
        },
        endQuestionTimer: (key: string, opts?: { publish?: boolean }) => {
            log(`endQuestionTimer(${isBlock(key) ? key : "local"}, ${JSON.stringify(opts ?? null)})`);
            if (!isBlock(key)) real.endQuestionTimer(key, opts);
        },
        noteQuestionActivity: (key: string) => {
            log(`noteQuestionActivity(${isBlock(key) ? key : "local"})`);
            if (!isBlock(key)) real.noteQuestionActivity(key);
        },
        reconcileQuestionTimer: (key: string) => {
            log(`reconcileQuestionTimer(${isBlock(key) ? key : "local"})`);
        },
        releaseQuestionTimer: (key: string) => {
            log(`releaseQuestionTimer(${isBlock(key) ? key : "local"})`);
            if (!isBlock(key)) real.releaseQuestionTimer(key);
        },
        questionCountdown: (key: string) => (isBlock(key) ? null : real.questionCountdown(key)),
    };
});

// `agent:askquestiontimeoutms`, set per scenario.
let timeoutSetting: unknown;
vi.mock("@/app/store/global", async (importOriginal) => ({
    ...(await importOriginal<typeof import("@/app/store/global")>()),
    getSettingsKeyAtom: (key: string) => () => (key === "agent:askquestiontimeoutms" ? timeoutSetting : undefined),
}));

const SNAP = (name: string) => `./__snapshots__/AgentQuestionPanel.behavior.${name}.txt`;

const question = (
    id: string,
    questions: Array<{ question: string; header: string; multiSelect: boolean; options: Array<{ label: string; description?: string }> }>
): ToolNode =>
    ({
        type: "tool",
        id,
        tool: "Other",
        params: {},
        status: "awaiting_answer",
        collapsed: false,
        summary: "❓ Waiting for your answer",
        question: { type: "ask_user_question", tool_use_id: id, questions },
    }) as ToolNode;

const colour = (id = "q1") =>
    question(id, [
        {
            question: "Pick a colour",
            header: "Colour",
            multiSelect: false,
            options: [{ label: "Red" }, { label: "Blue (Recommended)", description: "Calmer" }],
        },
    ]);
const several = (id = "q2") =>
    question(id, [
        { question: "Pick a colour", header: "Colour", multiSelect: false, options: [{ label: "Red" }, { label: "Blue" }] },
        {
            question: "Pick toppings",
            header: "Toppings",
            multiSelect: true,
            options: [{ label: "Ham (Recommended)" }, { label: "Egg" }, { label: "Cheese (recommended)" }],
        },
        { question: "Anything else", header: "Extra", multiSelect: false, options: [] },
    ]);

beforeEach(() => {
    T.length = 0;
    expiries.clear();
    timeoutSetting = undefined;
    realTimer.resetQuestionTimersForTests();
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
    realTimer.resetQuestionTimersForTests();
    document.body.innerHTML = "";
});

function mount(initial: ToolNode[], blockId?: string) {
    const [pending, setPending] = createSignal<ToolNode[]>(initial);
    const props = {
        pending,
        onAnswer: (o: unknown) => {
            log(`onAnswer ${JSON.stringify(o)}`);
        },
        onCancel: (id: string) => {
            log(`onCancel ${id}`);
        },
    };
    const r = render(() => (
        <div class="agent-view">
            <textarea data-testid="composer" />
            <AgentQuestionPanel {...props} blockId={blockId} />
        </div>
    ));
    return { setPending, unmount: r.unmount };
}

const panel = () => document.querySelector(".agent-question-panel") as HTMLElement | null;

/** What the panel shows right now, as lines. */
function view(label: string) {
    log(`-- ${label}`);
    const p = panel();
    if (!p) {
        log("  (no panel)");
        return;
    }
    const queue = p.querySelector(".agent-question-panel-queue")?.textContent;
    const cd = p.querySelector(".agent-question-panel-countdown");
    log(`  header: ${p.querySelector(".agent-question-panel-title")?.textContent}${queue ? ` | ${queue}` : ""}${cd ? ` | ${cd.textContent} [${cd.className}]` : ""}`);
    for (const q of Array.from(p.querySelectorAll("fieldset"))) {
        log(`  Q: ${q.querySelector("legend")?.textContent}`);
        for (const o of Array.from(q.querySelectorAll(".agent-question-panel-option"))) {
            const input = o.querySelector("input") as HTMLInputElement;
            const desc = o.querySelector(".agent-question-panel-option-desc")?.textContent;
            log(
                `    - ${o.querySelector(".agent-question-panel-option-label")?.textContent}${desc ? ` / ${desc}` : ""} {${input.type},${input.checked ? "checked" : "-"}${o.classList.contains("agent-question-panel-option--recommended") ? ",recommended" : ""}} name=${input.name}`
            );
        }
        log(`    other: "${(q.querySelector("input[type=text], .agent-question-panel-other-input") as HTMLInputElement)?.value}"`);
    }
    const buttons = Array.from(p.querySelectorAll(".agent-question-panel-actions button")).map(
        (b) => `${b.textContent}${(b as HTMLButtonElement).disabled ? " (disabled)" : ""}`
    );
    log(`  buttons: ${buttons.join(" | ")}`);
}

const option = (label: RegExp) =>
    Array.from(document.querySelectorAll(".agent-question-panel-option"))
        .map((o) => o.querySelector("input") as HTMLInputElement)
        .find((i) => label.test(i.closest("label")?.textContent ?? ""))!;
const others = () => Array.from(document.querySelectorAll(".agent-question-panel-other-input")) as HTMLInputElement[];
const button = (name: RegExp) =>
    Array.from(document.querySelectorAll(".agent-question-panel-actions button")).find((b) =>
        name.test(b.textContent ?? "")
    ) as HTMLButtonElement;
const click = (el: HTMLElement) => {
    fireEvent.click(el);
};
const type = (el: HTMLInputElement, text: string) => {
    fireEvent.input(el, { target: { value: text } });
};
const key = (el: Element | Window, k: string, init: KeyboardEventInit = {}) => {
    log(`key ${k}${init.shiftKey ? "+shift" : ""}${init.repeat ? " (repeat)" : ""} on ${(el as HTMLElement).tagName ?? "window"}${(el as HTMLElement).dataset?.testid ? `[${(el as HTMLElement).dataset.testid}]` : ""}`);
    el.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true, ...init }));
};

describe("AgentQuestionPanel behavior", () => {
    it("answers: single and multiple choice, Other, and what is sent", async () => {
        mount([several()]);
        view("opened: nothing chosen");
        click(option(/^Red/));
        view("Red");
        type(others()[0], "Green");
        view("typing Other supersedes the radio choice");
        click(option(/^Blue/));
        view("choosing a radio clears Other");
        click(option(/Ham/));
        click(option(/Egg/));
        view("two toppings");
        click(option(/Ham/));
        view("Ham toggled off");
        type(others()[1], "Olives");
        view("Other on a multi-select keeps the choices");
        type(others()[1], "   ");
        view("a blank Other counts for nothing");
        type(others()[2], "A note");
        view("everything answered");
        click(button(/Submit answer/));
        type(others()[2], "");
        view("Extra emptied");
        click(button(/Submit answer/));
        key(document.body, "Enter", { shiftKey: true });
        key(document.body, "Enter");
        await expectSnapshot("answers");
    });

    it("recommended: Accept Recommended overwrites, and the options without a recommended one", async () => {
        mount([several()]);
        fireEvent.click(option(/^Red/));
        fireEvent.click(option(/Egg/));
        view("two choices made by hand");
        fireEvent.click(button(/Accept Recommended/));
        view("after Accept Recommended");
        await expectSnapshot("recommended");
    });

    it("recommended: one question, the flagged option, and none flagged", async () => {
        const { setPending } = mount([colour()]);
        view("a flagged option");
        fireEvent.click(button(/Accept Recommended/));
        setPending([question("q9", [{ question: "Size", header: "Size", multiSelect: false, options: [{ label: "S" }, { label: "L" }] }])]);
        view("nothing flagged: the first option is the recommended one");
        fireEvent.click(button(/Accept Recommended/));
        expect(recommendedOptions([]).length).toBe(0);
        await expectSnapshot("recommended-first");
    });

    it("times out: the recommended defaults fill what was not answered, and the next question starts afresh", async () => {
        vi.useFakeTimers();
        const { setPending } = mount([several("q1"), colour("q2")]);
        view("counting");
        vi.advanceTimersByTime(21_000);
        view("21s in");
        fireEvent.click(option(/^Red/));
        view("answered one by hand");
        vi.advanceTimersByTime(9_000);
        view("the timeout fired");
        setPending([colour("q2")]);
        view("the next question");
        vi.advanceTimersByTime(30_000);
        view("and it times out too");
        setPending([]);
        view("nothing pending");
        await expectSnapshot("timeout");
    });

    it("times out on the configured time, and falls back when the setting is not a positive number", async () => {
        vi.useFakeTimers();
        timeoutSetting = 5000;
        const { setPending } = mount([colour("q1")]);
        view("5s configured");
        vi.advanceTimersByTime(5_000);
        for (const bad of [0, -3, "soon", undefined]) {
            timeoutSetting = bad;
            setPending([colour(`q-${String(bad)}`)]);
            log(`setting = ${JSON.stringify(bad) ?? "undefined"}`);
            view("a new question");
            vi.advanceTimersByTime(30_000);
        }
        await expectSnapshot("timeout-setting");
    });

    it("cancels from the button and from Escape, but not from an editable control elsewhere", async () => {
        mount([several("q1")]);
        fireEvent.click(button(/Cancel/));
        key(document.body, "Escape");
        const composer = screen.getByTestId("composer");
        key(composer, "Escape");
        key(composer, "Enter");
        key(others()[0], "Escape");
        key(panel()!, "Escape");
        key(others()[0], "Enter");
        key(document.body, "a");
        view("after all that");
        await expectSnapshot("cancel");
    });

    it("counts a real mouse move, a key and focus as activity, but not a parked pointer or a held key", async () => {
        vi.useFakeTimers();
        mount([colour()]);
        fireEvent.pointerEnter(panel()!, { clientX: 5, clientY: 5 });
        view("pointer entered");
        fireEvent.pointerMove(panel()!, { clientX: 5, clientY: 5 });
        view("a move to the same place");
        fireEvent.pointerMove(panel()!, { clientX: 9, clientY: 5 });
        view("a real move hides the countdown");
        vi.advanceTimersByTime(15_000);
        view("15s later it is back");
        key(screen.getByTestId("composer"), "a");
        key(screen.getByTestId("composer"), "a", { repeat: true });
        view("a key, then a held key");
        fireEvent.focusIn(option(/^Red/));
        view("focus inside the panel");
        fireEvent.focusIn(screen.getByTestId("composer"));
        view("focus outside the panel is ignored");
        fireEvent.pointerMove(panel()!, { clientX: 20, clientY: 5 });
        fireEvent.pointerMove(panel()!, { clientX: 20, clientY: 5 });
        await expectSnapshot("activity");
    });

    it("publishes a block's timer, keeps it reconciled, and lets go on unmount", async () => {
        const { setPending, unmount } = mount([colour("q1")], "blk-1");
        view("mounted with a block");
        setPending([colour("q2")]);
        view("next question");
        expiries.get("blk-1")?.();
        view("it expired");
        setPending([]);
        view("nothing pending");
        setPending([colour("q3")]);
        fireEvent.click(button(/Cancel/));
        unmount();
        log("unmounted");
        await expectSnapshot("published");
    });

    it("mounts with nothing pending and clears a stale timer key", async () => {
        const { setPending } = mount([], "blk-2");
        view("nothing pending");
        setPending([colour("q1")]);
        view("a question arrives");
        await expectSnapshot("empty");
    });

    it("lays out the queue and the options' descriptions", async () => {
        mount([colour("q1"), several("q2"), colour("q3")]);
        view("three queued");
        await expectSnapshot("queue");
    });
});

function expectSnapshot(name: string) {
    return expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));
}
