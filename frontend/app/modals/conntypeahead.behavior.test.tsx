// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ChangeConnectionBlockModal (the "Connect to" picker of a pane), pinned as a whole ahead of splitting it into a state
 * object and item builders: each scenario mounts the real component with stand-ins for the stores it reads (the pane's
 * block, the connection statuses, the config, the remotes), the RPCs it calls, the key helpers and the typeahead modal
 * (which draws the suggestions it is given, with their highlighted row, and has a button for each callback), and logs
 * the modal's props as a tree and, in order, every lookup, RPC and store write. The logs are snapshots recorded on the
 * unsplit component; the split must leave them byte for byte.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal, For, type JSX } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const T: string[] = [];
const log = (line: string): void => {
    T.push(line);
};

const h = vi.hoisted(() => ({
    block: null as any,
    status: null as any,
    allStatus: null as any,
    config: null as any,
    remotes: null as any,
    s3: [] as string[] | Error,
    showS3: false,
    ensureFails: false,
    typed: "",
}));

vi.mock("@/app/block/blockutil", () => ({ computeConnColorNum: (s?: { activeconnnum?: number }) => s?.activeconnnum ?? 0 }));
vi.mock("@/app/store/global", () => ({
    atoms: { allConnStatus: () => h.allStatus[0](), fullConfigAtom: () => h.config[0]() },
    getConnStatusAtom: (conn: string) => (log(`getConnStatusAtom ${conn}`), () => h.status[0]()),
    getHostName: () => "box",
    getUserName: () => "me",
    MOS: {
        makeORef: (type: string, id: string) => `${type}:${id}`,
        useMuxObjectValue: (oref: string) => (log(`useMuxObjectValue ${oref}`), [() => h.block[0]()]),
    },
}));
vi.mock("@/app/store/keymodel", () => ({ globalRefocusWithTimeout: (ms: number) => log(`globalRefocusWithTimeout ${ms}`) }));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ConnConnectCommand: (_c: unknown, d: unknown, o: unknown) => (log(`ConnConnectCommand ${JSON.stringify(d)} ${JSON.stringify(o)}`), Promise.resolve()),
        ConnDisconnectCommand: (_c: unknown, conn: string, o: unknown) => (log(`ConnDisconnectCommand ${conn} ${JSON.stringify(o)}`), Promise.resolve()),
        ConnListAWSCommand: (_c: unknown, o: unknown) => (
            log(`ConnListAWSCommand ${JSON.stringify(o)}`),
            h.s3 instanceof Error ? Promise.reject(h.s3) : Promise.resolve(h.s3)
        ),
        ConnEnsureCommand: (_c: unknown, d: unknown, o: unknown) => (
            log(`ConnEnsureCommand ${JSON.stringify(d)} ${JSON.stringify(o)}`),
            h.ensureFails ? Promise.reject(new Error("ensure failed")) : Promise.resolve()
        ),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/layout/index", () => ({}));
vi.mock("@/util/keyutil", () => ({
    checkKeyPressed: (e: { key: string }, key: string) => e.key === key,
    keydownWrapper: (f: (e: unknown) => boolean) => (e: unknown) => (log("keydownWrapper"), f(e)),
}));
vi.mock("@/app/store/block-meta", () => ({ setBlockMeta: (id: string, meta: unknown) => (log(`setBlockMeta ${id} ${JSON.stringify(meta)}`), Promise.resolve()) }));
vi.mock("@/app/store/remotes-store", () => ({
    refreshRemotes: () => (log("refreshRemotes"), Promise.resolve()),
    remotesList: () => (log("remotesList"), h.remotes[0]),
}));
vi.mock("@/app/view/remotes/open-remotes", () => ({
    openRemotesInPane: (id: string, conn?: string) => (log(`openRemotesInPane ${id} ${conn}`), Promise.resolve()),
}));
vi.mock("./conn-remote-items", () => ({
    remoteSuggestionScopes: (remotes: string[], typed: string, conn: string | undefined, color: (n: string) => number) => {
        log(`remoteSuggestionScopes ${JSON.stringify(remotes)} typed=${JSON.stringify(typed)} conn=${conn}`);
        const items = remotes
            .filter((r) => r.includes(typed))
            .map((name) => ({ status: "connected", icon: "server", iconColor: "var(--grey-text-color)", value: name, label: name, current: name === conn, color: color(name) }));
        return items.length ? [{ headerText: "SSH hosts", items }] : [];
    },
}));
vi.mock("@/app/modals/typeaheadmodal", () => ({
    TypeAheadModal: (p: Record<string, any>) => {
        const flat = () => (p.suggestions as any[]).flatMap((s) => ("items" in s ? s.items : [s]));
        const brief = (i: any) => ({ label: i.label, value: i.value, icon: i.icon, iconColor: i.iconColor, status: i.status, current: i.current, hasOnSelect: !!i.onSelect });
        const shown = () => JSON.stringify((p.suggestions as any[]).map((s) => ("items" in s ? { headerText: s.headerText, items: s.items.map(brief) } : brief(s))));
        const key = (k: string) => (
            <span data-cb={`key-${k}`} onClick={() => log(`onKeyDown ${k} -> ${p.onKeyDown({ key: k })}`)}>
                {k}
            </span>
        );
        return (
            <div
                data-stand="TypeAheadModal"
                data-suggestions={shown()}
                data-select-index={p.selectIndex}
                data-auto-focus={String(p.autoFocus)}
                data-value={p.value}
                data-label={p.label}
            >
                <span data-cb="onSelect" onClick={() => p.onSelect(h.typed)}>
                    onSelect
                </span>
                <span data-cb="onChange" onClick={() => p.onChange(h.typed)}>
                    onChange
                </span>
                <span data-cb="onClickBackdrop" onClick={() => p.onClickBackdrop()}>
                    backdrop
                </span>
                {key("Enter")}
                {key("Escape")}
                {key("ArrowUp")}
                {key("ArrowDown")}
                {key("x")}
                <For each={flat()}>
                    {(item: any, i) => (
                        <span data-cb={`item-${i()}`} onClick={() => (log(`item ${item.label}`), item.onSelect?.(item.value))}>
                            {item.label}
                        </span>
                    )}
                </For>
            </div>
        );
    },
}));

import { ChangeConnectionBlockModal } from "./conntypeahead";

const SNAP = (name: string) => `./__snapshots__/conntypeahead.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const ATTRS = /^(data-.*)$/;

/** The modal's stand-in as an indented tree of its props. */
function dump(node: Node, depth = 0, out: string[] = []): string[] {
    const pad = "  ".repeat(depth);
    if (node.nodeType === Node.TEXT_NODE) return out;
    if (node.nodeType !== Node.ELEMENT_NODE) return out;
    const el = node as HTMLElement;
    if (el.dataset.cb) return out;
    const attrs = [...el.attributes].filter((a) => ATTRS.test(a.name)).map((a) => `${a.name}=${a.value}`);
    out.push(`${pad}<${el.tagName.toLowerCase()}${attrs.length ? " " + attrs.join(" ") : ""}>`);
    return out;
}

let host: HTMLElement;
let setOpen: (v: boolean) => void;
let setFocused: (v: boolean) => void;

const look = (label: string) => {
    log(`-- ${label}`);
    log(dump(host.firstElementChild ?? document.createComment("")).join("\n") || "-");
};
const flush = async () => {
    for (let i = 0; i < 4; i++) await new Promise((r) => setTimeout(r, 0));
};
const press = async (cb: string) => {
    fireEvent.click(host.querySelector(`[data-cb="${cb}"]`)!);
    await flush();
};
const type = async (v: string) => {
    h.typed = v;
    await press("onChange");
};

const conn = (connection: string, status: string, activeconnnum = 1) => ({ connection, status, activeconnnum });

async function mount(open = true) {
    const [isOpen, setIsOpen] = createSignal(open);
    const [focused, setIsFocused] = createSignal(true);
    setOpen = (v) => setIsOpen(v);
    setFocused = setIsFocused;
    host = render(() => (
        <ChangeConnectionBlockModal
            blockId="blk-1"
            viewModel={{ showS3: () => h.showS3 } as any}
            blockRef={{ current: null as any }}
            connBtnRef={{ current: null as any }}
            changeConnModalOpen={isOpen}
            setChangeConnModalOpen={(v) => (log(`setChangeConnModalOpen ${v}`), setIsOpen(v))}
            nodeModel={{ isFocused: focused } as any}
        />
    )).container;
    await flush();
}

beforeEach(() => {
    T.length = 0;
    h.typed = "";
    h.block = createSignal<any>({ meta: { connection: "ssh:dev", file: "~/work" } });
    h.status = createSignal<any>(conn("ssh:dev", "connected"));
    h.allStatus = createSignal<any[]>([conn("ssh:dev", "connected", 3), conn("ssh:old", "disconnected", 1)]);
    h.config = createSignal<any>({ connections: { "aws:hidden": { "display:hidden": true }, "aws:b": { "display:order": -1 } } });
    h.remotes = createSignal<string[]>(["ssh:dev", "ssh:old"]);
    h.s3 = [];
    h.showS3 = false;
    h.ensureFails = false;
    vi.spyOn(console, "log").mockImplementation((...a: unknown[]) => log(`console.log ${a.map((x) => String(x)).join(" ")}`));
});
afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
});

describe("ChangeConnectionBlockModal, pinned", () => {
    it("builds its lookups in order, lists local, the remotes, disconnect and manage remotes for a connected pane, and loads the S3 list when opened", async () => {
        await mount();
        look("open, connected to ssh:dev");
        await press("onClickBackdrop");
        return snapshot("layout");
    });

    it("stays empty when it is closed as it is made, and loads the S3 list only once it is opened", async () => {
        await mount(false);
        look("closed");
        setOpen(true);
        await flush();
        look("opened later: still nothing drawn, but the lists are loaded");
        return snapshot("closed");
    });

    it("filters by what is typed, offers a new connection for an unknown name and a reconnect when the pane is down", async () => {
        h.status[1](conn("ssh:dev", "error"));
        await mount();
        look("pane in error: reconnect offered");
        await type("old");
        look("typed old");
        await type("brand-new");
        look("typed an unknown name");
        await type("me@box");
        look("typed the local name");
        await type("");
        await press("item-0");
        await press("item-1");
        return snapshot("typed");
    });

    it("runs the disconnect and manage-remotes items, and the new-connection item", async () => {
        await mount();
        const labels = [...host.querySelectorAll("[data-cb^=item-]")].map((e) => e.textContent);
        log(`items: ${JSON.stringify(labels)}`);
        await press(`item-${labels.findIndex((l) => l?.startsWith("Disconnect"))}`);
        await press(`item-${labels.findIndex((l) => l === "Manage remotes…")}`);
        await type("fresh-host");
        const fresh = [...host.querySelectorAll("[data-cb^=item-]")].map((e) => e.textContent);
        await press(`item-${fresh.findIndex((l) => l?.endsWith("(New Connection)"))}`);
        return snapshot("items");
    });

    it("lists the S3 profiles when the view shows them, leaving out the hidden ones and sorting by the configured order, and moves the file to the root", async () => {
        h.showS3 = true;
        h.s3 = ["aws:a", "aws:hidden", "aws:b"];
        await mount();
        look("with S3");
        await type("aws:");
        look("typed aws:");
        const labels = [...host.querySelectorAll("[data-cb^=item-]")].map((e) => e.textContent);
        await press(`item-${labels.findIndex((l) => l === "aws:b")}`);
        h.s3 = new Error("no aws");
        setOpen(false);
        setOpen(true);
        await flush();
        return snapshot("s3");
    });

    it("moves the highlighted row with the arrows, runs it on Enter (its own action or a plain change), clears on Escape and resets on any other key", async () => {
        await mount();
        await press("key-ArrowDown");
        look("down");
        await press("key-ArrowDown");
        await press("key-ArrowDown");
        await press("key-ArrowDown");
        await press("key-ArrowDown");
        await press("key-ArrowDown");
        look("down past the end");
        await press("key-ArrowUp");
        await press("key-Enter");
        look("after enter");
        await press("key-ArrowDown");
        await press("key-x");
        look("after another key");
        await press("key-Escape");
        look("after escape");
        await type("brand-new");
        await press("key-Enter");
        setFocused(false);
        look("not focused");
        return snapshot("keys");
    });

    it("changes the connection: nothing when it is the same one, null for the local one, the file to ~ or / or empty, and logs a failed connect", async () => {
        await mount();
        h.typed = "ssh:dev";
        await press("onSelect");
        const select = async (name: string) => {
            h.typed = name;
            fireEvent.click(host.querySelector(`[data-cb="onSelect"]`)!);
            await flush();
        };
        await select("ssh:dev");
        await select("");
        await select("aws:prod");
        h.block[1]({ meta: { connection: "ssh:dev", file: "" } });
        await select("ssh:new");
        h.block[1]({ meta: { connection: "ssh:dev", file: "~/x" } });
        h.ensureFails = true;
        await select("ssh:broken");
        return snapshot("change");
    });
});
