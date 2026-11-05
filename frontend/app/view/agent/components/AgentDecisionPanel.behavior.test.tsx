// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentDecisionPanel, pinned as a whole before it was split into hook groups: it had no test of its own.
 * Each scenario drives the panel (scope, Allow and the high-risk confirm, Deny with feedback, Defer and
 * Escape, the reset when the head request changes) and logs, in order, what the DOM shows and what reaches
 * `onDecide` / `onDefer`. The logs are snapshots recorded on the unsplit component; the split must leave
 * them byte for byte.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PermissionRequestEvent, ToolNode } from "../../../agent-model/types";
import { AgentDecisionPanel } from "./AgentDecisionPanel";

const T: string[] = [];
const log = (line: string) => T.push(line);
const SNAP = (name: string) => `./__snapshots__/AgentDecisionPanel.behavior.${name}.txt`;
const flush = async () => {
    for (let i = 0; i < 4; i++) await Promise.resolve();
};

const node = (id: string, p: Partial<PermissionRequestEvent> = {}): ToolNode =>
    ({
        type: "tool",
        id,
        tool: "Other",
        params: {},
        status: "pending_approval",
        collapsed: false,
        summary: "waiting",
        pendingPermission: { type: "permission_request", request_id: id, tool: "bash", target: "rm -rf build", ...p },
    }) as ToolNode;

beforeEach(() => {
    T.length = 0;
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
    document.body.innerHTML = "";
});

function mount(initial: ToolNode[], opts: { defer?: boolean } = { defer: true }) {
    const [pending, setPending] = createSignal<ToolNode[]>(initial);
    render(() => (
        <div class="agent-view">
            <textarea data-testid="composer" />
            <AgentDecisionPanel
                pending={pending}
                onDecide={(d) => {
                    log(`onDecide ${JSON.stringify(d)}`);
                }}
                onDefer={
                    opts.defer
                        ? () => {
                              log("onDefer");
                          }
                        : undefined
                }
            />
        </div>
    ));
    return { setPending };
}

const mask = (s: string | null | undefined) => (s ?? "").replace(/agent-decision-(scope|deny-error)-\S+/g, "agent-decision-$1-<uid>");

/** What the panel shows right now, as lines. */
function view(label: string) {
    log(`-- ${label}`);
    const full = document.querySelector(".agent-decision-panel");
    const min = document.querySelector(".agent-decision-panel-minimized");
    if (!full && !min) {
        log("  (nothing)");
        return;
    }
    if (min) {
        log(`  minimized [${min.className}] "${min.textContent}" aria=${min.getAttribute("aria-label")}`);
        return;
    }
    const p = full!;
    log(`  panel [${p.className}] role=${p.getAttribute("role")} aria=${p.getAttribute("aria-label")} tabindex=${p.getAttribute("tabindex")}`);
    const queue = p.querySelector(".agent-decision-panel-queue")?.textContent?.trim();
    const risk = p.querySelector(".agent-decision-panel-risk")?.textContent;
    log(`  header: ${p.querySelector(".agent-decision-panel-title")?.textContent?.trim()}${queue ? ` | ${queue}` : ""}${risk ? ` | ${risk}` : ""}`);
    log(`  meta: ${Array.from(p.querySelectorAll(".agent-decision-panel-meta > *")).map((e) => e.textContent).join(" / ")}`);
    const pre = p.querySelector(".agent-decision-panel-preview");
    log(`  preview: ${pre ? JSON.stringify(pre.textContent) : "-"}`);
    for (const l of Array.from(p.querySelectorAll(".agent-decision-panel-scope-option"))) {
        const input = l.querySelector("input") as HTMLInputElement;
        log(`  scope ${l.querySelector(".agent-decision-panel-scope-label")?.textContent} / ${l.querySelector(".agent-decision-panel-scope-hint")?.textContent} {${input.checked ? "checked" : "-"}${l.classList.contains("agent-decision-panel-scope-option--active") ? ",active" : ""}} name=${mask(input.name)}`);
    }
    const fb = p.querySelector(".agent-decision-panel-feedback-input") as HTMLTextAreaElement | null;
    if (fb) {
        const err = p.querySelector(".agent-decision-panel-feedback-error");
        log(
            `  feedback "${fb.value}" [${fb.className}] invalid=${fb.getAttribute("aria-invalid")} describedby=${mask(fb.getAttribute("aria-describedby")) || "-"} focused=${document.activeElement === fb}${err ? ` error="${err.textContent}" id=${mask(err.id)} role=${err.getAttribute("role")}` : ""}`
        );
    }
    const buttons = Array.from(p.querySelectorAll(".agent-decision-panel-actions button")).map(
        (b) => `${b.textContent}${b.getAttribute("aria-pressed") != null ? `[pressed=${b.getAttribute("aria-pressed")}]` : ""}${b.getAttribute("title") ? `(title: ${b.getAttribute("title")})` : ""}`
    );
    log(`  buttons: ${buttons.join(" | ")}`);
}

const key = (el: Element, k: string, init: KeyboardEventInit = {}) => {
    log(`key ${k}${init.shiftKey ? "+shift" : ""}${init.ctrlKey ? "+ctrl" : ""} on ${el.tagName}${(el as HTMLElement).dataset?.testid ? `[${(el as HTMLElement).dataset.testid}]` : ""}`);
    el.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true, ...init }));
};
const composer = () => document.querySelector("[data-testid=composer]") as HTMLElement;
const panel = () => document.querySelector(".agent-decision-panel") as HTMLElement;
const button = (re: RegExp) =>
    Array.from(document.querySelectorAll("button")).find((b) => re.test(b.textContent ?? "")) as HTMLButtonElement;
const radio = (label: string) =>
    Array.from(document.querySelectorAll(".agent-decision-panel-scope-option"))
        .find((l) => l.querySelector(".agent-decision-panel-scope-label")?.textContent === label)!
        .querySelector("input") as HTMLInputElement;
const feedback = () => document.querySelector(".agent-decision-panel-feedback-input") as HTMLTextAreaElement;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

describe("AgentDecisionPanel behavior", () => {
    it("lays out the request: meta, previews, queue and risk", async () => {
        const { setPending } = mount([]);
        view("nothing pending");
        setPending([node("a", { risk: "low", reason: "cleanup", preview: { kind: "bash", command: "rm -rf build" } }), node("b"), node("c")]);
        view("three queued, the head low risk with a bash preview");
        setPending([node("d", { risk: "high", target: null, preview: { kind: "diff", content: "-a\n+b" } })]);
        view("high risk, no target, a diff");
        setPending([node("e", { preview: { kind: "text", content: "plain" }, reason: "why" })]);
        view("medium by default, a text preview");
        setPending([node("f", { preview: { kind: "none" } })]);
        view("no preview");
        setPending([node("g", { preview: undefined })]);
        view("preview absent");
        await snapshot("layout");
    });

    it("sets the scope with the radios and with letter keys, but not while typing", async () => {
        mount([node("a")]);
        fireEvent.click(radio("Project"));
        view("Project by radio");
        for (const k of ["o", "S", "p", "G", "x", "ss"]) key(document.body, k);
        view("after o S p G x ss on the body");
        key(composer(), "o");
        view("o in the composer is left alone");
        key(document.body, "s", { ctrlKey: true });
        view("ctrl+s counts as s");
        await snapshot("scope");
    });

    it("allows: Enter, the high-risk confirm, a shift click, and keys from elsewhere", async () => {
        vi.useFakeTimers();
        const { setPending } = mount([node("a", { risk: "low" })]);
        key(document.body, "p");
        key(document.body, "Enter");
        view("after Enter on a low-risk request");
        setPending([node("b", { risk: "high" })]);
        view("a high-risk request");
        key(document.body, "Enter");
        view("first Enter arms it");
        key(document.body, "Enter");
        view("second Enter allows");
        setPending([node("c", { risk: "high" })]);
        key(document.body, "Enter");
        vi.advanceTimersByTime(499);
        view("armed, 499ms later");
        vi.advanceTimersByTime(1);
        view("armed, 500ms later: disarmed");
        key(document.body, "Enter");
        fireEvent.click(button(/Press again|Allow/), { shiftKey: true });
        view("a shift click allows at once");
        setPending([node("d", { risk: "medium" })]);
        key(composer(), "Enter");
        view("Enter in the composer is left to the composer");
        fireEvent.click(button(/^Allow$/));
        key(panel(), "Enter");
        view("Allow button and Enter inside the panel");
        await snapshot("allow");
    });

    it("denies with feedback: the scope rules, the error, Back, and the textarea's keys", async () => {
        const { setPending } = mount([node("a")]);
        key(document.body, "Enter", { shiftKey: true });
        await flush();
        view("Shift+Enter opens the feedback");
        fireEvent.click(button(/Send denial/));
        view("once with no feedback is sent");
        setPending([node("b")]);
        fireEvent.click(button(/Deny \+ feedback/));
        await flush();
        fireEvent.click(radio("Session"));
        fireEvent.click(button(/Send denial/));
        view("session with no feedback is refused");
        fireEvent.input(feedback(), { target: { value: "no" } });
        view("typing clears the error");
        key(feedback(), "Enter");
        view("Enter in the textarea does nothing");
        key(feedback(), "Enter", { shiftKey: true });
        view("Shift+Enter in the textarea sends");
        setPending([node("c")]);
        fireEvent.click(button(/Deny \+ feedback/));
        fireEvent.input(feedback(), { target: { value: "  because  " } });
        key(document.body, "Enter");
        view("Enter on the body in deny mode sends, trimmed");
        setPending([node("d")]);
        fireEvent.click(button(/Deny \+ feedback/));
        fireEvent.input(feedback(), { target: { value: "draft" } });
        fireEvent.click(button(/Back/));
        view("Back clears the draft");
        fireEvent.click(button(/Deny \+ feedback/));
        view("and the feedback is empty again");
        await snapshot("deny");
    });

    it("minimizes on Escape and Defer, and comes back from the bar", async () => {
        const { setPending } = mount([node("a", { risk: "high" }), node("b")]);
        key(composer(), "Escape");
        view("Escape in the composer is left alone");
        key(document.body, "Escape");
        view("Escape on the body minimizes");
        fireEvent.click(document.querySelector(".agent-decision-panel-minimized")!);
        view("re-opened from the bar");
        key(panel(), "Escape");
        view("Escape inside the panel minimizes");
        fireEvent.click(document.querySelector(".agent-decision-panel-minimized")!);
        fireEvent.click(button(/Defer/));
        view("the Defer button minimizes");
        setPending([node("c", { risk: "low" })]);
        view("a new head request re-opens it");
        await snapshot("defer");
    });

    it("has no Defer button, and Escape only minimizes, without onDefer", async () => {
        mount([node("a")], { defer: false });
        view("no onDefer");
        key(document.body, "Escape");
        view("after Escape");
        await snapshot("no-defer");
    });

    it("resets every transient flag when the head request changes", async () => {
        vi.useFakeTimers();
        const { setPending } = mount([node("a", { risk: "high" })]);
        fireEvent.click(radio("Global"));
        key(document.body, "Enter");
        view("armed, scope Global");
        setPending([node("b", { risk: "high" })]);
        view("the next request: not armed, scope Once");
        vi.advanceTimersByTime(600);
        key(document.body, "Enter");
        view("armed afresh by one Enter");
        key(document.body, "Enter", { shiftKey: true });
        fireEvent.input(feedback(), { target: { value: "draft" } });
        fireEvent.click(radio("Project"));
        fireEvent.click(button(/Send denial/));
        view("a denial of this one");
        setPending([node("c", { risk: "high" })]);
        view("the next request: nothing carried over");
        await snapshot("reset");
    });

    it("scopes its ids to the instance", async () => {
        mount([node("a")]);
        mount([node("b")]);
        const names = Array.from(document.querySelectorAll(".agent-decision-panel-scope-option input")).map((i) => (i as HTMLInputElement).name);
        log(`radio names: ${names.length}, distinct per panel: ${new Set(names).size === 2}`);
        await snapshot("ids");
    });
});
