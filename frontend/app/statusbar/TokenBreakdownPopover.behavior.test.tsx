// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The token usage popover, pinned as a whole ahead of splitting it into a state object and view parts: each scenario
 * mounts the real popover over the real token-usage store (turns are recorded into it), with stand-ins for the anchored
 * popover, the confirm modal and the ambient purposes and a controllable ambient summary, and logs the popover as an
 * indented tree and, in order, every call it makes (the ambient watch, the pane reveal, the close, the reset). The logs
 * are snapshots recorded on the unsplit component; the split must leave them byte for byte.
 * (TokenBreakdownPopover.test.tsx asserts what the person sees; this pins the markup and the calls.)
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { recordTurn, resetSession } from "@/store/token-usage";

const h = vi.hoisted(() => ({
    log: [] as string[],
    summary: null as unknown,
}));

vi.mock("@/app/store/ambient-health", async (orig) => {
    const actual = await orig<typeof import("@/app/store/ambient-health")>();
    return {
        ...actual,
        ambientSummary: () => (globalThis as any).__ambientSummary?.() ?? null,
        watchAmbient: (ms: number) => {
            h.log.push(`watchAmbient ${ms}`);
            return () => h.log.push("unwatchAmbient");
        },
    };
});
vi.mock("@/app/util/reveal-block", () => ({
    revealBlock: (id: string, opts: unknown) => (h.log.push(`revealBlock ${id} ${JSON.stringify(opts)}`), Promise.resolve()),
}));
vi.mock("@/app/element/anchored-popover", () => ({
    AnchoredPopover: (props: Record<string, any>) => (
        <div
            data-stand="AnchoredPopover"
            data-placement={props.placement}
            data-class={props.class}
            data-role={props.role}
            data-label={props["aria-label"]}
            data-style={JSON.stringify(props.style)}
        >
            <span data-cb="onDismiss" onClick={() => props.onDismiss()}>
                dismiss
            </span>
            {props.children}
        </div>
    ),
}));
vi.mock("./AmbientPurposes", () => ({ AmbientPurposes: () => <div data-stand="AmbientPurposes" /> }));
// The time of day is local; the snapshots must not be.
vi.spyOn(Date.prototype, "toLocaleTimeString").mockImplementation(function (this: Date) {
    return "T";
});

import { TokenBreakdownPopover } from "./TokenBreakdownPopover";

const T = h.log;
const log = (line: string) => T.push(line);
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(`./__snapshots__/TokenBreakdownPopover.behavior.${name}.txt`);

const ATTRS = /^(class|role|aria-.*|data-.*|title|disabled|type|href)$/;
/** The popover as an indented tree: tags, classes, the attributes that carry state, and text. */
function dump(node: Node, depth = 0, out: string[] = []): string[] {
    const pad = "  ".repeat(depth);
    if (node.nodeType === Node.TEXT_NODE) {
        const t = (node.textContent ?? "").trim().replace(/\s+/g, " ");
        if (t) out.push(`${pad}"${t}"`);
        return out;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return out;
    const el = node as HTMLElement;
    if (el.tagName === "svg" || el.tagName === "STYLE") return out;
    const attrs = [...el.attributes].filter((a) => ATTRS.test(a.name)).map((a) => `${a.name}=${JSON.stringify(a.value)}`);
    out.push(`${pad}<${el.tagName.toLowerCase()}${attrs.length ? " " + attrs.join(" ") : ""}>`);
    el.childNodes.forEach((c) => dump(c, depth + 1, out));
    return out;
}

let host: HTMLElement;
const look = (label: string) => {
    log(`-- ${label}`);
    log(dump(host).join("\n"));
};
const flush = async () => {
    for (let i = 0; i < 4; i++) await new Promise((r) => setTimeout(r, 0));
};
const [summary, setSummary] = createSignal<unknown>(null);
(globalThis as any).__ambientSummary = summary;

function mount() {
    const view = render(() => <TokenBreakdownPopover anchor={null} onClose={() => log("onClose")} />);
    host = view.container;
    return view;
}
const button = (text: string) => [...host.querySelectorAll("button")].find((b) => b.textContent?.includes(text)) as HTMLElement;
const cb = (stand: string, name: string) => host.querySelector(`[data-stand="${stand}"] [data-cb="${name}"]`)!;

const claude = (over: Record<string, unknown> = {}) => ({ input: 1000, output: 50, freshInput: 200, cacheCreation: 0, cacheRead: 800, ...over });

beforeEach(() => {
    T.length = 0;
    resetSession();
    setSummary(null);
});
afterEach(() => {
    cleanup();
    resetSession();
});

describe("TokenBreakdownPopover, pinned", () => {
    it("says no turn has completed yet, with the reset button off", async () => {
        mount();
        await flush();
        look("empty");
        await snapshot("empty");
    });

    it("lists each agent with turns, cost and cache share, then the total and the session cache rate", async () => {
        recordTurn("claude", claude(), { blockId: "b1", agentName: "Manoz", costUsd: 0.1234, numTurns: 1 });
        recordTurn("claude", claude({ input: 500, output: 20, freshInput: 500, cacheRead: 0 }), { blockId: "b1", agentName: "Manoz", costUsd: 0.05, numTurns: 1 });
        recordTurn("codex", { input: 300, output: 10 }, { blockId: "b2", agentName: "Codex Agent", numTurns: 1 });
        recordTurn("claude", { input: 70, output: 3 }, { agentName: "No pane", costUsd: 0, numTurns: 3 } as any);
        mount();
        await flush();
        look("agents");
        await snapshot("agents");
    });

    it("collapses the internal bucket, expands it to its services, and lists a failing purpose when nothing else ran", async () => {
        recordTurn("claude", claude(), { blockId: "b1", agentName: "Manoz" });
        recordTurn("ambient:next_prompt_suggestion", { input: 100, output: 5 });
        recordTurn("ambient:activity_summary", { input: 100, output: 5 });
        recordTurn("ambient:zeta_thing", { input: 400, output: 5 });
        mount();
        await flush();
        look("collapsed");
        fireEvent.click(button("AgentMux internal"));
        await flush();
        look("expanded");
        fireEvent.click(button("AgentMux internal"));
        await flush();
        look("collapsed again");
        await snapshot("ambient");
    });

    it("shows the purposes alone when only the ambient summary has anything to say", async () => {
        setSummary({ unhealthyCount: 1 });
        mount();
        await flush();
        look("summary only");
        setSummary(null);
        await flush();
        look("summary gone");
        await snapshot("summary-only");
    });

    it("reveals an agent's pane and closes, and does nothing for a row with no pane", async () => {
        recordTurn("claude", claude(), { blockId: "b1", agentName: "Manoz", numTurns: 1 });
        recordTurn("claude", claude(), { agentName: "No pane", numTurns: 1 } as any);
        mount();
        await flush();
        fireEvent.click(button("Manoz"));
        fireEvent.click(button("No pane"));
        fireEvent.click(cb("AnchoredPopover", "onDismiss"));
        await flush();
        await snapshot("clicks");
    });

    it("asks inside the panel before resetting, and resets and closes only when confirmed", async () => {
        recordTurn("claude", claude(), { blockId: "b1", agentName: "Manoz", numTurns: 1 });
        mount();
        await flush();
        fireEvent.click(button("Reset counter"));
        await flush();
        look("asking");
        expect(host.querySelector('[role="group"][aria-label="Reset token counter?"]')).not.toBeNull();
        expect(document.activeElement).toBe(button("Cancel"));
        fireEvent.click(button("Cancel"));
        await flush();
        look("cancelled");
        fireEvent.click(button("Reset counter"));
        fireEvent.click(button("Reset"));
        await flush();
        look("reset");
        await snapshot("reset");
    });

    it("keeps an agent's row element as turns land, and drops the row of an agent that leaves", async () => {
        recordTurn("claude", claude(), { blockId: "b1", agentName: "Manoz", numTurns: 1 });
        mount();
        await flush();
        const before = button("Manoz");
        recordTurn("claude", claude(), { blockId: "b1", agentName: "Manoz", numTurns: 1 });
        recordTurn("claude", claude(), { blockId: "b2", agentName: "Camper", numTurns: 1 });
        await flush();
        log(`same row element: ${button("Manoz") === before}`);
        look("two agents");
        resetSession();
        await flush();
        look("after a reset from elsewhere");
        await snapshot("live");
    });

    it("watches the ambient counts while it is open and stops when it closes", async () => {
        const view = mount();
        await flush();
        view.unmount();
        await snapshot("watch");
    });
});
