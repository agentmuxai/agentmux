// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentSiteGrants (Settings › Browser › Agents on sites), pinned as a whole ahead of splitting it into a state object
 * and view parts: each scenario mounts the real section over a stand-in domain store and RPC whose answers the
 * scenario controls, with stand-ins for the kit's button, notice, select and text input, drives the checkboxes,
 * fields and buttons, and logs the section as an indented tree and, in order, every call it makes (the store
 * following and holding a value, and each grant command with its request). The logs are snapshots recorded on the
 * unsplit component; the split must leave them byte for byte.
 * (agent-site-grants.test.tsx asserts what the person sees against the real kit; this pins the markup and the calls.)
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal, type JSX } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const T: string[] = [];
const log = (line: string): void => {
    T.push(line);
};

const [data, setData] = createSignal<any>(null);
const [storeError, setStoreError] = createSignal<string | null>(null);

const h = vi.hoisted(() => ({
    opts: null as any,
    select: null as any,
    results: {} as Record<string, () => Promise<any>>,
}));

vi.mock("@/app/store/domain-store", () => ({
    createDomainStore: (opts: any) => {
        h.opts = opts;
        return {
            data: () => data(),
            error: () => storeError(),
            loaded: () => data() !== null,
            set: (v: unknown) => (log(`store.set ${JSON.stringify(v)}`), setData(() => v)),
            refresh: async () => {},
            follow: () => log("store.follow"),
        };
    },
}));
vi.mock("@/app/store/rpc-api", () => {
    const cmd = (name: string) => (_client: unknown, req?: unknown) => (
        log(`${name} ${JSON.stringify(req ?? null)}`),
        h.results[name]()
    );
    return {
        RpcApi: {
            BrowserGrantsListCommand: cmd("BrowserGrantsListCommand"),
            BrowserGrantsSetCommand: cmd("BrowserGrantsSetCommand"),
            BrowserGrantsPresetCommand: cmd("BrowserGrantsPresetCommand"),
            BrowserGrantsRevokeCommand: cmd("BrowserGrantsRevokeCommand"),
            BrowserGrantsPowersCommand: cmd("BrowserGrantsPowersCommand"),
            ListAgentDefinitionsCommand: cmd("ListAgentDefinitionsCommand"),
        },
    };
});
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/element/ui", () => ({
    Button: (
        p: { disabled?: boolean; title?: string; onClick?: () => void; children?: JSX.Element } & Record<
            string,
            unknown
        >
    ) => (
        <span
            data-stand="Button"
            data-disabled={String(p.disabled)}
            data-title={p.title}
            data-aria-label={p["aria-label"] as string | undefined}
            onClick={() => {
                log(`Button ${JSON.stringify(p["aria-label"] ?? p.children)} click`);
                if (!p.disabled) p.onClick?.();
            }}
        >
            {p.children}
        </span>
    ),
    Notice: (p: { tone?: string; title?: JSX.Element; children?: JSX.Element }) => (
        <div data-stand="Notice" data-tone={p.tone}>
            <b>{p.title}</b>
            {p.children}
        </div>
    ),
    Select: (p: {
        ariaLabel?: string;
        value?: string;
        onChange?: (v: string) => void;
        options: { value: string; label: string }[];
    }) => {
        h.select = p;
        return (
            <div data-stand="Select" data-aria-label={p.ariaLabel} data-value={p.value}>
                {p.options.map((o) => (
                    <span data-option={o.value}>{o.label}</span>
                ))}
            </div>
        );
    },
    TextInput: (p: Record<string, any>) => (
        <input
            data-stand="TextInput"
            data-detached={String(p.detached)}
            aria-label={p["aria-label"]}
            placeholder={p.placeholder}
            data-value={p.value}
            value={p.value}
            onInput={p.onInput}
            onKeyDown={p.onKeyDown}
        />
    ),
}));
vi.mock("@/app/view/browser/bypass", () => ({
    LATE_PROMPT_LIMIT: "A line already on its way can't be stopped.",
    siteLabel: (origin: string) => origin.replace(/^https?:\/\//, ""),
}));

import { AgentSiteGrants } from "./agent-site-grants";

const SNAP = (name: string) => `./__snapshots__/agent-site-grants.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const ATTRS = /^(class|role|aria-.*|data-.*|title|style|disabled|type|value|placeholder|id)$/;

/** The section as an indented tree: tags, classes, the attributes that carry state (checked included), and text. */
function dump(node: Node, depth = 0, out: string[] = []): string[] {
    const pad = "  ".repeat(depth);
    if (node.nodeType === Node.TEXT_NODE) {
        const t = (node.textContent ?? "").trim();
        if (t) out.push(`${pad}"${t}"`);
        return out;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return out;
    const el = node as HTMLElement;
    const attrs = [...el.attributes]
        .filter((a) => ATTRS.test(a.name))
        .map((a) => `${a.name}=${JSON.stringify(a.value)}`);
    if (el instanceof HTMLInputElement && el.type === "checkbox") attrs.push(`checked=${el.checked}`);
    out.push(`${pad}<${el.tagName.toLowerCase()}${attrs.length ? " " + attrs.join(" ") : ""}>`);
    el.childNodes.forEach((c) => dump(c, depth + 1, out));
    return out;
}

let host: HTMLElement;
const look = (label: string) => {
    log(`-- ${label}`);
    log(dump(host).join("\n") || "-");
};
const flush = async () => {
    for (let i = 0; i < 12; i++) await Promise.resolve();
};
const defer = <T,>() => {
    let resolve!: (v: T) => void;
    let reject!: (e: unknown) => void;
    const promise = new Promise<T>((res, rej) => ((resolve = res), (reject = rej)));
    return { promise, resolve, reject };
};
const button = (label: string) =>
    [...host.querySelectorAll<HTMLElement>("[data-stand='Button']")].find(
        (x) => x.dataset.ariaLabel === label || x.textContent?.trim() === label
    )!;
const click = (label: string) => fireEvent.click(button(label));
const checkbox = (label: string) => host.querySelector<HTMLInputElement>(`input[aria-label='${label}']`)!;
const site = () => host.querySelector<HTMLInputElement>("input[aria-label='Site']")!;
const typeSite = (v: string) => fireEvent.input(site(), { target: { value: v } });
const pick = (slug: string) => h.select.onChange(slug);
/** The two terminal checkboxes of the add form, by their text. */
const terminalBox = (text: string) =>
    [...host.querySelectorAll<HTMLLabelElement>(".agent-site-grants-add label")]
        .find((l) => l.textContent?.includes(text))!
        .querySelector("input")!;

const NOW = 1_800_000_000_000;
const agent = (slug: string, name: string, extra: Record<string, unknown> = {}) => ({
    id: `id-${slug}`,
    slug,
    name,
    ...extra,
});
const AGENTS = [
    agent("naki-2", "Naki-2", { updated_at: 10 }),
    agent("agent1", "Agent1", { last_active_at: 900 }),
    agent("nameless", "", { updated_at: 5 }),
];
const row = (over: Record<string, unknown> = {}) => ({
    id: "r1",
    agent: "naki-2",
    origins: ["https://shell.cloud.google.com", "https://ssh.cloud.google.com"],
    powers: ["terminal"],
    granted_at_ms: NOW - 3 * 3600_000,
    ...over,
});
const PRESET = {
    id: "google-cloud-shell",
    label: "Google Cloud Shell",
    sites: ["https://shell.cloud.google.com"],
    note: "Not yet checked.",
};
const grants = (rows: unknown[] = [row()], over: Record<string, unknown> = {}) => ({
    rows,
    presets: [PRESET],
    powers: ["terminal", "terminal_ask"],
    ...over,
});
const loaded = (rows?: unknown[], over?: Record<string, unknown>) =>
    setData({ grants: grants(rows, over), agents: AGENTS });

function mount() {
    const view = render(() => <AgentSiteGrants />);
    host = view.container;
    return view;
}

beforeEach(() => {
    T.length = 0;
    h.select = null;
    setData(null);
    setStoreError(null);
    for (const name of [
        "BrowserGrantsSetCommand",
        "BrowserGrantsPresetCommand",
        "BrowserGrantsRevokeCommand",
        "BrowserGrantsPowersCommand",
    ]) {
        h.results[name] = async () => grants([row({ id: "r2" })]);
    }
    h.results.BrowserGrantsListCommand = async () => grants();
    h.results.ListAgentDefinitionsCommand = async () => AGENTS;
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(NOW);
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
});

describe("AgentSiteGrants, pinned", () => {
    it("follows the store, says it is loading until the grants arrive (and not when there is an error), shows the load's error, and loads grants and the agents together", async () => {
        mount();
        look("loading");
        setStoreError("srv is down");
        look("the load failed");
        setStoreError(null);
        loaded();
        look("loaded");
        log(`store: events=${JSON.stringify(h.opts.events)} initial=${JSON.stringify(h.opts.initial)}`);
        log(`load -> ${JSON.stringify(await h.opts.load())}`);
        return snapshot("load");
    });

    it("lists each grant with the agent's name (matched by slug, in any case, or the slug itself), its sites, when it was given, and a checkbox for each power the row can carry", async () => {
        loaded(
            [
                row({ granted_at_ms: NOW - 10_000 }),
                row({
                    id: "r2",
                    agent: "AGENT1",
                    origins: ["https://example.com"],
                    powers: [],
                    granted_at_ms: NOW - 5 * 60_000,
                }),
                row({
                    id: "r3",
                    agent: "gone",
                    origins: [],
                    powers: ["terminal", "terminal_ask"],
                    granted_at_ms: NOW - 2 * 86_400_000,
                }),
                row({
                    id: "r4",
                    agent: "nameless",
                    origins: ["https://a.dev"],
                    powers: ["mystery"],
                    granted_at_ms: NOW - 3_600_000,
                }),
            ],
            { powers: ["terminal", "terminal_ask", "mystery"] }
        );
        mount();
        look("four grants");
        cleanup();
        T.push("-- no grants, no consoles");
        loaded([], { presets: [] });
        mount();
        look("no grants");
        return snapshot("rows");
    });

    it("changes a row's powers from its checkboxes, revokes it, shows busy while a change is out, and says why a change failed (and drops that when the next one works)", async () => {
        loaded([
            row(),
            row({ id: "r2", agent: "agent1", powers: ["terminal", "terminal_ask"], origins: ["https://example.com"] }),
        ]);
        mount();
        const out = defer<any>();
        h.results.BrowserGrantsPowersCommand = () => out.promise;
        fireEvent.click(
            checkbox("Terminal: ask me for each line for Naki-2 on shell.cloud.google.com (and ssh.cloud.google.com)")
        );
        look("a power change is out");
        click("Revoke Agent1 on example.com");
        out.resolve(grants([row({ id: "r9" })]));
        await flush();
        look("changed");
        h.results.BrowserGrantsPowersCommand = async () => {
            throw new Error("not allowed");
        };
        fireEvent.click(
            checkbox("Terminal: type and run commands for Naki-2 on shell.cloud.google.com (and ssh.cloud.google.com)")
        );
        await flush();
        look("the change failed");
        h.results.BrowserGrantsRevokeCommand = async () => {
            throw "plain text";
        };
        click("Revoke Naki-2 on shell.cloud.google.com (and ssh.cloud.google.com)");
        await flush();
        look("the revoke failed with a string");
        h.results.BrowserGrantsRevokeCommand = async () => grants([]);
        click("Revoke Naki-2 on shell.cloud.google.com (and ssh.cloud.google.com)");
        await flush();
        look("revoked, the error gone");
        return snapshot("powers");
    });

    it("lets an agent onto a site: the agent picker (newest use first, name or slug), the terminal choices, Allow off until both are given, Enter to add, the site cleared when it worked and kept when it didn't", async () => {
        loaded();
        mount();
        look("the add form");
        pick("naki-2");
        typeSite("  shell.example.com ");
        look("an agent and a site");
        fireEvent.click(terminalBox("Terminal: type"));
        look("terminal on");
        fireEvent.click(terminalBox("show me each line"));
        look("terminal, ask first");
        fireEvent.click(terminalBox("show me each line"));
        fireEvent.click(terminalBox("Terminal: type"));
        look("terminal off again");
        click("Allow");
        await flush();
        look("added with no powers");
        pick("agent1");
        typeSite("a.example.com");
        fireEvent.click(terminalBox("Terminal: type"));
        fireEvent.click(terminalBox("show me each line"));
        fireEvent.keyDown(site(), { key: "a" });
        fireEvent.keyDown(site(), { key: "Enter" });
        await flush();
        look("added with Enter, ask first");
        h.results.BrowserGrantsSetCommand = async () => {
            throw new Error("that site is not allowed");
        };
        typeSite("bad.example.com");
        click("Allow");
        await flush();
        look("the add failed, the site kept");
        pick("");
        fireEvent.keyDown(site(), { key: "Enter" });
        look("Enter with no agent adds nothing");
        return snapshot("add");
    });

    it("turns on a console for the picked agent, and asks for an agent first", async () => {
        loaded();
        mount();
        click("Google Cloud Shell");
        look("no agent picked");
        pick("agent1");
        look("an agent picked");
        click("Google Cloud Shell");
        await flush();
        look("turned on");
        h.results.BrowserGrantsPresetCommand = async () => {
            throw new Error("no such console");
        };
        click("Google Cloud Shell");
        await flush();
        look("the console failed");
        return snapshot("presets");
    });

    it("holds nothing when a change comes back before the grants have loaded", async () => {
        mount();
        const out = defer<any>();
        h.results.BrowserGrantsRevokeCommand = () => out.promise;
        loaded();
        click("Revoke Naki-2 on shell.cloud.google.com (and ssh.cloud.google.com)");
        setData(null);
        out.resolve(grants([]));
        await flush();
        look("grants went away while the change was out");
        return snapshot("late");
    });
});
