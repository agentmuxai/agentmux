// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The header strip of an agent-driven browser pane, pinned as a whole ahead of splitting it into a state object and
 * view parts: each scenario feeds it a browser model made of signals (the grant, the pause, the log, the sites) and
 * logs the strip as a tree and, in order, every call its buttons make. The logs are snapshots recorded on the
 * unsplit component; the split must leave them byte for byte. (driven-by-strip.test.tsx asserts what the person is
 * told; this pins the markup and the calls.)
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    log: [] as string[],
    all: { paused: false, since_ms: null as number | null, by: null as string | null },
    resumeAll: (async () => {}) as (p: boolean) => Promise<void>,
}));

vi.mock("@/app/store/browser-pause-all", () => ({
    browserPauseAll: () => h.all,
    pausedLabel: (p: { by: string | null }) => `Agent browsing paused by ${p.by ?? "you"}`,
    setAgentBrowsingPaused: (paused: boolean) => {
        h.log.push(`setAgentBrowsingPaused ${paused}`);
        return h.resumeAll(paused);
    },
}));
// The time of day is local; the snapshots must not be.
vi.mock("./action-log", async (orig) => ({
    ...(await orig<typeof import("./action-log")>()),
    actionTime: (ts: number) => `t${ts}`,
}));

import type { BrowserViewModel } from "./browser-model";
import { ActionLogStrip, DrivenByStrip } from "./driven-by-strip";

const T = h.log;
const log = (line: string) => T.push(line);
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(`./__snapshots__/driven-by-strip.behavior.${name}.txt`);

const ATTRS = /^(class|role|aria-.*|data-.*|title|disabled|type|href)$/;
/** The strip as an indented tree: tags, classes, the attributes that carry state, and text. */
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
    for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
};
const gate = () => {
    let open!: () => void;
    let fail!: (e: unknown) => void;
    const promise = new Promise<void>((res, rej) => ((open = res), (fail = rej)));
    return { promise, open, fail };
};

const GRANT = { agent: "Naki", origins: ["https://github.com", "https://*.example.com"], minutes: 60, expires_at: 1, always: false, powers: [] as string[] };
const entry = (ts: number, extra: Record<string, unknown> = {}) => ({
    ts,
    agent: "Naki",
    kind: "click",
    target: "Submit",
    detail: undefined as string | undefined,
    how: "done",
    url: "https://github.com/agentmuxai/agentmux?tab=issues",
    ...extra,
});

/** A browser model made of signals, whose calls are logged and can be held or refused. */
function fakeModel(initial: Record<string, any> = {}) {
    const [bypass, setBypassMeta] = createSignal<any>(initial.bypass);
    const [ended, setEnded] = createSignal<string | undefined>(initial.ended);
    const [actions, setActions] = createSignal<any>(initial.actions ?? { count: 0, entries: [] });
    const [paused, setPaused] = createSignal<boolean>(initial.paused ?? false);
    const [origins, setOrigins] = createSignal<string[]>(initial.origins ?? []);
    const [bypassPaused, setBypassPaused] = createSignal<boolean>(initial.bypassPaused ?? false);
    const [minutesLeft, setMinutesLeft] = createSignal<number>(initial.minutesLeft ?? 42);
    const [preview, setPreview] = createSignal<string[]>(initial.preview ?? ["https://github.com"]);
    const calls = {
        setBypass: (async () => {}) as (on: boolean, minutes?: number) => Promise<void>,
        setPaused: (async () => {}) as (p: boolean) => Promise<void>,
        takeOver: (async () => {}) as () => Promise<void>,
    };
    const model = {
        actionsAtom: actions,
        pausedAtom: paused,
        allowedOriginsAtom: origins,
        tookControlAtom: () => false,
        bypassAtom: bypass,
        bypassEndedAtom: ended,
        bypassPaused,
        bypassMinutesLeft: minutesLeft,
        bypassOriginsPreview: preview,
        setBypass: (on: boolean, minutes?: number) => {
            log(`model.setBypass ${on}${minutes === undefined ? "" : " " + minutes}`);
            return calls.setBypass(on, minutes);
        },
        setPaused: (p: boolean) => {
            log(`model.setPaused ${p}`);
            return calls.setPaused(p);
        },
        takeOver: () => {
            log("model.takeOver");
            return calls.takeOver();
        },
    } as unknown as BrowserViewModel;
    return { model, calls, setBypassMeta, setEnded, setActions, setPaused, setOrigins, setBypassPaused, setMinutesLeft, setPreview };
}

function mount(m: ReturnType<typeof fakeModel>, agent = "Naki") {
    const view = render(() => <DrivenByStrip model={m.model} agent={agent} />);
    host = view.container;
}
const button = (text: string) => [...host.querySelectorAll("button")].find((b) => b.textContent?.trim() === text || b.textContent?.trim().startsWith(text + " ("))!;
const press = async (text: string, detail = 1) => {
    fireEvent.click(button(text), { detail });
    await flush();
};

beforeEach(() => {
    T.length = 0;
    h.all = { paused: false, since_ms: null, by: null };
    h.resumeAll = async () => {};
});
afterEach(() => cleanup());

describe("Driven-by strip: the plain strip", () => {
    it("says who drives the pane, the sites it is limited to, and why auto-approve ended", () => {
        const m = fakeModel();
        mount(m);
        look("plain");
        m.setOrigins(["https://github.com"]);
        look("limited to one site");
        m.setOrigins(["https://github.com", "https://example.com", "https://*.x.org"]);
        look("limited to three sites");
        m.setEnded("the page left github.com");
        look("auto-approve ended");
        return snapshot("plain");
    });

    it("Take over ends the agent's control, and a refusal is swallowed", async () => {
        const m = fakeModel();
        m.calls.takeOver = () => Promise.reject(new Error("no"));
        mount(m);
        await press("Take over");
        look("after a refused take over");
        return snapshot("take-over");
    });
});

describe("Driven-by strip: pause", () => {
    it("Pause holds the agent from any press, Resume lets it act only on a pointer click", async () => {
        const m = fakeModel();
        mount(m);
        await press("Pause", 0);
        m.setPaused(true);
        look("paused");
        await press("Resume", 0);
        await press("Resume", 1);
        m.setPaused(false);
        look("resumed");
        return snapshot("pause");
    });

    it("while every pane is paused the strip says so, and Resume lifts it for every pane", async () => {
        h.all = { paused: true, since_ms: 1, by: "Posa" };
        const m = fakeModel({ paused: true });
        mount(m);
        look("paused for every pane");
        await press("Resume", 0);
        await press("Resume", 1);
        h.resumeAll = () => Promise.reject(new Error("srv said no"));
        await press("Resume", 1);
        look("lifting it failed");
        return snapshot("pause-all");
    });

    it("a failed pause or resume says so in the strip, and the next try clears it", async () => {
        const m = fakeModel();
        m.calls.setPaused = () => Promise.reject(new Error("srv said no"));
        mount(m);
        await press("Pause", 1);
        look("pause failed");
        const g = gate();
        m.calls.setPaused = () => g.promise;
        await press("Pause", 1);
        look("trying again");
        g.fail(new Error("still no"));
        await flush();
        m.setPaused(true);
        m.calls.setPaused = () => Promise.reject(new Error("could not resume"));
        await press("Resume", 1);
        look("resume failed");
        m.calls.setPaused = () => Promise.reject(new Error("read from srv: connection attempt failed (os error 10060)"));
        await press("Resume", 1);
        look("resume failed, unreachable");
        return snapshot("pause-errors");
    });
});

describe("Driven-by strip: turning auto-approve on", () => {
    it("Auto-approve opens a confirmation with the sites and a duration, and again closes it", async () => {
        const m = fakeModel({ preview: ["https://github.com", "https://*.example.com"] });
        mount(m);
        await press("Auto-approve");
        look("confirming");
        await press("4 hours");
        look("4 hours picked");
        await press("15 min");
        await press("Cancel");
        look("cancelled");
        await press("Auto-approve");
        await press("Auto-approve");
        look("toggled closed");
        return snapshot("confirm");
    });

    it("with no site to cover it asks for a page first and cannot be turned on", async () => {
        const m = fakeModel({ preview: [] });
        mount(m);
        await press("Auto-approve");
        look("no sites");
        await press("Turn on auto-approve");
        return snapshot("confirm-no-sites");
    });

    it("grants only on a pointer click, once at a time, and closes on success", async () => {
        const m = fakeModel();
        mount(m);
        await press("Auto-approve");
        await press("4 hours");
        await press("Turn on auto-approve", 0);
        const g = gate();
        m.calls.setBypass = () => g.promise;
        await press("Turn on auto-approve", 1);
        look("busy");
        await press("Turn on auto-approve", 1);
        g.open();
        await flush();
        look("granted");
        return snapshot("grant");
    });

    it("a refused grant stays on the confirmation with why, and reopening clears it", async () => {
        const m = fakeModel();
        m.calls.setBypass = () => Promise.reject(new Error("srv said no"));
        mount(m);
        await press("Auto-approve");
        await press("Turn on auto-approve", 1);
        look("refused");
        m.calls.setBypass = () => Promise.reject(new Error("read from srv: A connection attempt failed (os error 10060)"));
        await press("Turn on auto-approve", 1);
        look("unreachable");
        await press("Cancel");
        await press("Auto-approve");
        look("reopened");
        return snapshot("grant-refused");
    });

    it("the confirmation goes away when the grant arrives from elsewhere", async () => {
        const m = fakeModel();
        mount(m);
        await press("Auto-approve");
        m.setBypassMeta(GRANT);
        await flush();
        look("grant arrived");
        return snapshot("grant-arrived");
    });
});

describe("Driven-by strip: while auto-approve is on", () => {
    it("shows the red strip with the sites, the powers and the time left, and what changes when paused", () => {
        const m = fakeModel({ bypass: GRANT });
        mount(m);
        look("on");
        m.setBypassMeta({ ...GRANT, always: true, powers: ["terminal"] });
        look("always, with terminal");
        m.setBypassMeta({ ...GRANT, powers: ["terminal_ask"] });
        look("terminal, asking");
        m.setBypassPaused(true);
        look("paused: the page left the sites");
        m.setBypassPaused(false);
        m.setPaused(true);
        m.setMinutesLeft(17);
        look("paused by the person, clock stopped");
        m.setBypassMeta({ ...GRANT, always: true });
        h.all = { paused: true, since_ms: 1, by: null };
        look("paused for every pane, held");
        return snapshot("bypass-on");
    });

    it("Stop auto-approve ends it, and a failure shows in the strip with its lead-in", async () => {
        const m = fakeModel({ bypass: GRANT });
        mount(m);
        await press("Stop auto-approve");
        m.calls.setBypass = () => Promise.reject(new Error("srv said no"));
        await press("Stop auto-approve");
        look("failed");
        m.calls.setBypass = () => Promise.reject(new Error("read from srv: connection attempt failed (os error 10060)"));
        await press("Stop auto-approve");
        look("unreachable");
        await press("Take over");
        return snapshot("bypass-stop");
    });
});

describe("Driven-by strip: the log", () => {
    it("the Log button counts the actions and opens the list, newest first, and Close shuts it", async () => {
        const entries = [
            entry(1),
            entry(2, { kind: "auto-approve-on", target: undefined, detail: "60 min", how: "approved" }),
            entry(3, { how: "refused: terminal", detail: "rm -rf" }),
            entry(4, { how: "declined", target: "", url: undefined }),
            entry(5, { how: "timed out", url: "not a url at all, and a very long one that has to be cut somewhere around here" }),
        ];
        const m = fakeModel({ actions: { count: 5, entries } });
        mount(m);
        look("closed");
        await press("Log");
        look("open");
        m.setActions({ count: 9, entries });
        look("the latest 5 of 9");
        await press("Close");
        look("closed again");
        return snapshot("log");
    });

    it("says Nothing yet for an empty log", async () => {
        const m = fakeModel();
        mount(m);
        await press("Log");
        look("empty");
        return snapshot("log-empty");
    });

    it("the log opens over the red strip too", async () => {
        const m = fakeModel({ bypass: GRANT, actions: { count: 1, entries: [entry(1)] } });
        mount(m);
        await press("Log");
        look("on, log open");
        return snapshot("log-bypass");
    });
});

describe("Action log strip", () => {
    it("names whoever drove last, defaults to The agent, and opens the log", async () => {
        const m = fakeModel();
        const view = render(() => <ActionLogStrip model={m.model} />);
        host = view.container;
        look("no entries");
        m.setActions({ count: 2, entries: [entry(1, { agent: "Naki" }), entry(2, { agent: "Posa" })] });
        look("two entries");
        await press("Log");
        look("open");
        m.setActions({ count: 2, entries: [entry(1), entry(2, { agent: "" })] });
        look("last entry without an agent");
        await press("Log");
        look("closed");
        return snapshot("strip");
    });
});
