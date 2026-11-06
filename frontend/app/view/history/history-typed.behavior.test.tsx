// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * History → Typed, pinned as a whole ahead of splitting it into a state object and view parts: each scenario
 * drives the pane (showing it, switching agent, searching, paging, forgetting, clearing, restoring, failing and
 * racing reads) and logs the pane as a tree and, in order, every call it makes to srv and to the composers'
 * shared cache. The logs are snapshots recorded on the unsplit component; the split must leave them byte for
 * byte. (history-typed.test.tsx asserts what the person sees; this pins the markup and the order of the calls.)
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    defs: [] as any[],
    defsError: null as string | null,
    pages: (() => []) as (agent: string, before: number | undefined) => any[],
    searchResult: [] as any[],
    trash: {} as Record<string, any[]>,
    historyError: null as string | null,
    cacheError: null as string | null,
    hold: {} as Record<string, Promise<void> | undefined>,
    log: [] as string[],
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListAgentDefinitionsCommand: async (_c: unknown, d: { include_hidden: boolean }) => {
            h.log.push(`rpc ListAgentDefinitions include_hidden=${d.include_hidden}`);
            if (h.defsError) throw new Error(h.defsError);
            return h.defs;
        },
        AgentInputHistoryCommand: async (_c: unknown, d: { agent_id: string; before_id?: number; limit: number }) => {
            h.log.push(`rpc AgentInputHistory agent=${d.agent_id} before=${d.before_id} limit=${d.limit}`);
            await h.hold[`history ${d.agent_id}`];
            if (h.historyError) throw new Error(h.historyError);
            return { entries: h.pages(d.agent_id, d.before_id) };
        },
        AgentInputHistorySearchCommand: async (_c: unknown, d: { agent_id: string; query: string; limit: number }) => {
            h.log.push(`rpc AgentInputHistorySearch agent=${d.agent_id} query=${JSON.stringify(d.query)} limit=${d.limit}`);
            if (h.historyError) throw new Error(h.historyError);
            return { entries: h.searchResult };
        },
        AgentInputHistoryTrashCommand: async (_c: unknown, d: { agent_id: string }) => {
            h.log.push(`rpc AgentInputHistoryTrash agent=${d.agent_id}`);
            await h.hold[`trash ${d.agent_id}`];
            return { batches: h.trash[d.agent_id] ?? [], retention_ms: 7 * 24 * 60 * 60 * 1000 };
        },
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/view/agent/agent-input-history", () => ({
    forgetInputHistoryEntry: async (agent: string, id: number) => {
        h.log.push(`cache forget agent=${agent} id=${id}`);
        await h.hold["forget"];
        if (h.cacheError) throw new Error(h.cacheError);
    },
    clearInputHistory: async (agent: string) => {
        h.log.push(`cache clear agent=${agent}`);
        await h.hold["clear"];
        if (h.cacheError) throw new Error(h.cacheError);
    },
    restoreInputHistory: async (batch: string) => {
        h.log.push(`cache restore batch=${batch}`);
        if (h.cacheError) throw new Error(h.cacheError);
        return 1;
    },
}));
// formatWhen formats in the machine's locale and zone; the snapshots must not.
vi.mock("./history-shared", async (orig) => ({
    ...(await orig<typeof import("./history-shared")>()),
    formatWhen: (ms: number) => `when:${ms}`,
}));

import { HistoryTyped } from "./history-typed";

const T = h.log;
const log = (line: string) => T.push(line);
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(`./__snapshots__/history-typed.behavior.${name}.txt`);

const ATTRS = /^(class|role|aria-.*|data-.*|title|disabled|type|href|value|selected|placeholder)$/;
/** The pane as an indented tree: tags, classes, the attributes that carry state, and text. */
function dump(node: Node, depth = 0, out: string[] = []): string[] {
    const pad = "  ".repeat(depth);
    if (node.nodeType === Node.TEXT_NODE) {
        const t = (node.textContent ?? "").trim();
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
    for (let i = 0; i < 8; i++) await vi.advanceTimersByTimeAsync(0);
};
const gate = () => {
    let open!: () => void;
    const promise = new Promise<void>((r) => (open = r));
    return { promise, open };
};

let setVisible: (v: boolean) => void;
async function mount(visible = true) {
    const view = render(() => {
        const [vis, set] = createSignal(visible);
        setVisible = set;
        return <HistoryTyped visible={vis()} />;
    });
    host = view.container;
    await flush();
    return view;
}
const click = async (name: string | RegExp, index = 0) => {
    const found = screen.getAllByRole("button", { name });
    fireEvent.click(found[index]);
    await flush();
};
const pickAgent = async (id: string) => {
    fireEvent.change(host.querySelector("select")!, { target: { value: id } });
    await flush();
};
const typeSearch = async (text: string) => {
    fireEvent.input(screen.getByPlaceholderText("Search what was typed"), { target: { value: text } });
    await flush();
};
const wait = async (ms: number) => {
    await vi.advanceTimersByTimeAsync(ms);
    await flush();
};

const item = (id: number, text: string, extra: Record<string, unknown> = {}) => ({
    id,
    agent_id: "a1",
    sent_at_ms: 1_000_000 + id * 60_000,
    text,
    kind: "message",
    source: "composer",
    sends: 1,
    ...extra,
});
const A1 = [
    item(5, "merge on approval", { sends: 3 }),
    item(4, "/compact", { kind: "slash" }),
    item(3, "git status", { kind: "bang", source: "swarm" }),
    item(2, "carry on", { source: "backfill" }),
    item(1, "hello"),
];
const batch = (id: string, count: number, sample: string, by = "korp") => ({
    batch_id: id,
    agent_id: "a1",
    deleted_at_ms: 9_000_000,
    deleted_by: by,
    count,
    sample,
});

beforeEach(() => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    T.length = 0;
    h.defs = [
        { id: "a2", name: "Beta" },
        { id: "a1", name: "Alpha" },
        { id: "zz23456789abcdef", name: "" },
    ];
    h.defsError = null;
    h.pages = (agent) => (agent === "a1" ? A1 : []);
    h.searchResult = [A1[0]];
    h.trash = {};
    h.historyError = null;
    h.cacheError = null;
    h.hold = {};
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
});

describe("History → Typed: loading", () => {
    it("loads nothing while hidden, then everything when it is shown, and stops reading when hidden again", async () => {
        await mount(false);
        look("hidden");
        setVisible(true);
        await flush();
        look("shown");
        setVisible(false);
        await flush();
        look("hidden again");
        setVisible(true);
        await flush();
        look("shown again");
        await snapshot("loading");
    });

    it("lists the agents by name (hidden ones too, an unnamed one by its id) and the entries with their tags and trash", async () => {
        h.trash = { a1: [batch("b-1", 1, "ship the fix"), batch("b-2", 3, "hello", "armory-ui")] };
        await mount();
        look("loaded");
        await snapshot("loaded");
    });

    it("says there is nothing yet, or no agent to show", async () => {
        h.pages = () => [];
        await mount();
        look("no entries");
        cleanup();
        T.length = 0;
        h.defs = [];
        await mount();
        look("no agents");
        await snapshot("empty");
    });
});

describe("History → Typed: agent and search", () => {
    it("switching agent reads that agent and its trash, and drops a question being asked", async () => {
        h.trash = { a1: [batch("b-1", 1, "ship the fix")] };
        await mount();
        await click("Forget", 1);
        look("asking about one entry");
        await click("Clear…");
        look("asking about both");
        await pickAgent("a2");
        look("after switching to a2");
        await pickAgent("a1");
        look("back on a1");
        await snapshot("switch-agent");
    });

    it("a search waits for a pause, only the last keystroke counts, and clearing it reads the newest page again", async () => {
        await mount();
        fireEvent.input(screen.getByPlaceholderText("Search what was typed"), { target: { value: "m" } });
        await wait(100);
        fireEvent.input(screen.getByPlaceholderText("Search what was typed"), { target: { value: "me" } });
        await wait(249);
        log("-- 249 ms after the last keystroke");
        await wait(1);
        log("-- 250 ms after the last keystroke");
        look("searched");
        await typeSearch("");
        look("search cleared");
        await snapshot("search");
    });

    it("a search with no match says so, and offers neither Clear… nor Show older", async () => {
        h.searchResult = [];
        await mount();
        await typeSearch("zzz");
        await wait(250);
        look("no match");
        await snapshot("search-no-match");
    });

    it("the Refresh button reads the agents, the page and the trash again, in that order", async () => {
        await mount();
        T.length = 0;
        await click("Refresh");
        look("refreshed");
        await snapshot("refresh");
    });
});

describe("History → Typed: paging", () => {
    it("offers older entries after a full page, reads before the oldest shown, and stops after a short page", async () => {
        const page = Array.from({ length: 200 }, (_, i) => item(500 - i, `msg ${500 - i}`));
        h.pages = (_a, before) => (before === undefined ? page : [item(300, "msg 300 older")]);
        await mount();
        look("first page");
        await click("Show older");
        look("older page");
        await snapshot("paging");
    });

    it("a full page of search results does not offer older entries", async () => {
        h.searchResult = Array.from({ length: 200 }, (_, i) => item(500 - i, `msg ${500 - i}`));
        await mount();
        await typeSearch("msg");
        await wait(250);
        log(`-- Show older buttons: ${screen.queryAllByRole("button", { name: "Show older" }).length}`);
        await snapshot("paging-search");
    });
});

describe("History → Typed: forgetting, clearing and restoring", () => {
    it("forgets one entry after a second click, and Cancel backs out", async () => {
        h.trash = { a1: [batch("b-1", 1, "ship the fix")] };
        await mount();
        await click("Forget", 2);
        look("asking");
        await click("Cancel");
        look("cancelled");
        await click("Forget", 4);
        await click("Forget this");
        look("forgotten");
        await snapshot("forget");
    });

    it("a refused forget shows why and keeps the entry", async () => {
        await mount();
        h.cacheError = "srv is busy";
        await click("Forget", 0);
        await click("Forget this");
        look("refused");
        await snapshot("forget-refused");
    });

    it("clears the agent's whole history after naming it and a second click, and Cancel backs out", async () => {
        await mount();
        await click("Clear…");
        look("asking");
        await click("Cancel");
        look("cancelled");
        await click("Clear…");
        await click(/Forget everything typed to/);
        look("cleared");
        await snapshot("clear");
    });

    it("names the agent by its short id when it has no name, and a refused clear shows why", async () => {
        h.defs = [{ id: "0123456789abcdef", name: "" }];
        h.pages = () => [item(1, "hello")];
        await mount();
        await click("Clear…");
        look("asking");
        h.cacheError = "no";
        await click(/Forget everything typed to/);
        look("refused");
        await snapshot("clear-refused");
    });

    it("a clear that finishes after switching agents does not empty the agent now shown", async () => {
        h.pages = (agent) => [item(agent === "a1" ? 3 : 9, agent === "a1" ? "for a1" : "for a2")];
        const g = gate();
        h.hold["clear"] = g.promise;
        await mount();
        await click("Clear…");
        await click(/Forget everything typed to/);
        await pickAgent("a2");
        look("on a2, the clear still pending");
        h.hold["clear"] = undefined;
        g.open();
        await flush();
        look("after the clear finished");
        await snapshot("clear-stale");
    });

    it("a forget that finishes after switching agents leaves the agent now shown alone", async () => {
        h.pages = (agent) => [item(agent === "a1" ? 3 : 9, agent === "a1" ? "for a1" : "for a2")];
        const g = gate();
        h.hold["forget"] = g.promise;
        await mount();
        await click("Forget", 0);
        await click("Forget this");
        await pickAgent("a2");
        g.open();
        await flush();
        look("after the forget finished");
        await snapshot("forget-stale");
    });

    it("Restore brings a batch back, reads the page and the trash again, and a refused one shows why", async () => {
        h.trash = { a1: [batch("b-1", 1, "ship the fix")] };
        await mount();
        T.length = 0;
        h.trash = { a1: [] };
        await click("Restore");
        look("restored");
        h.trash = { a1: [batch("b-2", 2, "again")] };
        await click("Refresh");
        h.cacheError = "gone";
        await click("Restore");
        look("refused");
        await snapshot("restore");
    });
});

describe("History → Typed: failures and races", () => {
    it("says why the agents can't be listed instead of staying on Loading, and Refresh tries again", async () => {
        h.defsError = "srv is busy";
        await mount();
        look("failed");
        h.defsError = null;
        await click("Refresh");
        look("refreshed");
        await snapshot("agents-failed");
    });

    it("says why the history can't be read, and the error clears when a read works", async () => {
        h.historyError = "srv is busy";
        await mount();
        look("failed");
        h.historyError = null;
        await click("Refresh");
        look("recovered");
        await snapshot("history-failed");
    });

    it("a read overtaken by another agent's does not land", async () => {
        h.pages = (agent) => [item(agent === "a1" ? 3 : 9, agent === "a1" ? "for a1" : "for a2")];
        const g = gate();
        h.hold["history a1"] = g.promise;
        await mount();
        look("a1 still loading");
        await pickAgent("a2");
        look("a2 shown");
        h.hold["history a1"] = undefined;
        g.open();
        await flush();
        look("a1 answered late");
        await snapshot("read-overtaken");
    });

    it("a trash read overtaken by another agent's does not land", async () => {
        h.trash = { a1: [batch("b-1", 1, "for a1")], a2: [batch("b-2", 1, "for a2")] };
        const g = gate();
        h.hold["trash a1"] = g.promise;
        await mount();
        await pickAgent("a2");
        look("a2 shown");
        h.hold["trash a1"] = undefined;
        g.open();
        await flush();
        look("a1 trash answered late");
        await snapshot("trash-overtaken");
    });
});
