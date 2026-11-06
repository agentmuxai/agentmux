// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Memory pane's Global section, pinned as a whole ahead of splitting it into a state object and view parts: each
 * scenario mounts the real manager over a view model made of signals, with stand-ins for the tile, the token strip,
 * the import banner and the full view, and logs the pane as an indented tree and, in order, every call it makes (the
 * model's open / close / move / restore / refresh, the drag session, the discard question). The logs are snapshots
 * recorded on the unsplit component; the split must leave them byte for byte. (global-bundle-manager.test.tsx asserts
 * what the person sees through the RPCs; this pins the markup and the calls.)
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal, type JSX } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Bundle } from "@/app/store/rpc-api";

const h = vi.hoisted(() => ({
    log: [] as string[],
    discard: true,
    model: null as any,
}));

vi.mock("@/app/drag/drag-session", async (orig) => {
    const actual = await orig<typeof import("@/app/drag/drag-session")>();
    return {
        ...actual,
        beginDrag: (...args: Parameters<typeof actual.beginDrag>) => {
            h.log.push(`beginDrag ${args[0]} ${JSON.stringify(args[1])}`);
            return actual.beginDrag(...args);
        },
        endDrag: (reason: Parameters<typeof actual.endDrag>[0]) => {
            h.log.push(`endDrag ${reason}`);
            return actual.endDrag(reason);
        },
    };
});
vi.mock("@/app/element/create-confirm", () => ({
    createConfirm: () => ({
        ask: (q: { title: string }) => {
            h.log.push(`confirm ${q.title}`);
            return Promise.resolve(h.discard);
        },
        Dialog: () => <div data-stand="ConfirmDialog" />,
    }),
}));
// The age of a file is read against the clock; the snapshots must not be.
vi.mock("@/app/view/native-memory/MemoryFileCard", () => ({ formatFileAge: (t?: number) => (t ? `age${t}` : "") }));
vi.mock("@/app/view/native-memory/MemoryTile", () => ({
    MemoryTile: (props: Record<string, any>) => {
        const shown = () => {
            const out: Record<string, string> = {};
            for (const k of Object.keys(props).sort()) {
                if (k === "classList" || k.startsWith("on")) continue;
                out[k] = typeof props[k] === "function" ? "<fn>" : JSON.stringify(props[k]) ?? "undefined";
            }
            return JSON.stringify(out);
        };
        return (
            <div
                data-stand="MemoryTile"
                data-props={shown()}
                data-handlers={Object.keys(props).filter((k) => k.startsWith("on")).sort().join(",")}
                classList={props.classList}
                draggable={props.draggable}
                onDragStart={props.onDragStart}
                onDragOver={props.onDragOver}
                onDragLeave={props.onDragLeave}
                onDrop={props.onDrop}
                onDragEnd={props.onDragEnd}
            >
                <span data-cb="onSelect" onClick={() => props.onSelect?.()}>
                    select
                </span>
            </div>
        );
    },
}));
vi.mock("./GlobalMemoryTokenStrip", () => ({
    GlobalMemoryTokenStrip: () => {
        h.log.push("TokenStrip created");
        return <div data-stand="TokenStrip" />;
    },
}));
vi.mock("./GlobalMemoryImportBanner", () => ({
    GlobalMemoryImportBanner: (props: { onImported: () => void }) => {
        h.log.push("ImportBanner created");
        return (
            <div data-stand="ImportBanner">
                <span data-cb="onImported" onClick={() => props.onImported()}>
                    imported
                </span>
            </div>
        );
    },
}));
vi.mock("./GlobalMemoryFullView", () => ({
    GlobalMemoryFullView: (props: { model: { viewAtom: () => { kind: string } | null } }) => {
        h.log.push("FullView created");
        return <div data-stand="FullView">{props.model.viewAtom()?.kind}</div>;
    },
}));
vi.mock("./global-bundle-model", async (orig) => {
    const actual = await orig<typeof import("./global-bundle-model")>();
    return {
        ...actual,
        GlobalBundleViewModel: class {
            constructor() {
                return h.model;
            }
        },
    };
});

import { GlobalBundleManager } from "./global-bundle-manager";

const T = h.log;
const log = (line: string) => T.push(line);
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(`./__snapshots__/global-bundle-manager.behavior.${name}.txt`);

const ATTRS = /^(class|role|aria-.*|data-.*|title|disabled|type|href|draggable)$/;
/** The pane as an indented tree: tags, classes, the attributes that carry state, and text. */
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

const entry = (id: string, over: Record<string, unknown> = {}): Bundle =>
    ({ id, name: `Entry ${id}`, instructions: `text of ${id}`, is_system: false, updated_at: 100, removed_by: "", removed_at: 0, ...over }) as unknown as Bundle;

/** A view model made of signals, whose calls are logged. */
function fakeModel(init: Record<string, any> = {}) {
    const [system, setSystem] = createSignal<Bundle[]>(init.system ?? []);
    const [ordinary, setOrdinary] = createSignal<Bundle[]>(init.ordinary ?? []);
    const [tokens, setTokens] = createSignal<Map<string, { tokens_est: number; size_bytes: number }>>(init.tokens ?? new Map());
    const [largest, setLargest] = createSignal<boolean>(init.largest ?? false);
    const [delivered, setDelivered] = createSignal<boolean>(init.delivered ?? false);
    const [kind, setKind] = createSignal<string>(init.kind ?? "host");
    const [totals, setTotals] = createSignal<{ total: number; count: number } | null>(init.totals ?? null);
    const [error, setError] = createSignal<string | null>(init.error ?? null);
    const [claude, setClaude] = createSignal<any>(init.claude ?? null);
    const [preview, setPreview] = createSignal<string>(init.preview ?? "");
    const [removed, setRemoved] = createSignal<Bundle[]>(init.removed ?? []);
    const [view, setView] = createSignal<any>(init.view ?? null);
    const [openEntry, setOpenEntry] = createSignal<Bundle | null>(init.openEntry ?? null);
    const [dirty, setDirty] = createSignal<boolean>(init.dirty ?? false);
    const model = {
        systemSectionsAtom: system,
        ordinarySectionsAtom: ordinary,
        tokensByIdAtom: tokens,
        largestFirstAtom: largest,
        deliveredAtom: delivered,
        agentKindAtom: kind,
        deliveredTotalsAtom: totals,
        errorAtom: error,
        claudeGlobalConfigAtom: claude,
        previewAtom: preview,
        removedAtom: removed,
        viewAtom: view,
        openEntryAtom: openEntry,
        draft: { dirtyAtom: dirty },
        open: (v: any) => {
            log(`model.open ${JSON.stringify(v)}`);
            setView(v);
        },
        close: () => {
            log("model.close");
            setView(null);
        },
        moveTo: (from: string, to: string) => {
            log(`model.moveTo ${from} ${to}`);
            return Promise.resolve();
        },
        restore: (e: Bundle) => {
            log(`model.restore ${e.id}`);
            return Promise.resolve();
        },
        refresh: () => {
            log("model.refresh");
            return Promise.resolve();
        },
        dispose: () => log("model.dispose"),
    };
    h.model = model;
    return { model, setSystem, setOrdinary, setTokens, setLargest, setDelivered, setKind, setTotals, setError, setClaude, setPreview, setRemoved, setView, setOpenEntry, setDirty };
}

function mount() {
    const view = render(() => <GlobalBundleManager />);
    host = view.container;
    return view;
}
const tiles = () => [...host.querySelectorAll<HTMLElement>('[data-stand="MemoryTile"]')];
const tileOf = (id: string) => tiles().find((t) => JSON.parse(t.getAttribute("data-props") ?? "{}").data?.includes(`"id":"${id}"`))!;
const choose = async (el: Element) => {
    fireEvent.click(el.querySelector("[data-cb]")!);
    await flush();
};
const dataTransfer = () => {
    const store: Record<string, string> = {};
    return { setData: (k: string, v: string) => (store[k] = v), getData: (k: string) => store[k] ?? "", effectAllowed: "", dropEffect: "" };
};

const TOKENS = new Map([
    ["s1", { tokens_est: 50, size_bytes: 200 }],
    ["o1", { tokens_est: 300, size_bytes: 1200 }],
    ["o2", { tokens_est: 150, size_bytes: 600 }],
]);

beforeEach(() => {
    T.length = 0;
    h.discard = true;
});
afterEach(() => {
    cleanup();
});

describe("GlobalBundleManager, pinned", () => {
    it("shows an empty Global Memory with its two action tiles", async () => {
        fakeModel();
        mount();
        await flush();
        look("empty");
        await snapshot("empty");
    });

    it("lays out system and ordinary entries in injection order, with their token share and age", async () => {
        fakeModel({
            system: [entry("s1", { is_system: true, name: "System rule" })],
            ordinary: [entry("o1"), entry("o2", { instructions: "  ", updated_at: 0 }), entry("o3", { instructions: "x" })],
            tokens: TOKENS,
            delivered: true,
            totals: { total: 500, count: 3 },
            preview: "héllo composed",
            kind: "container",
        });
        mount();
        await flush();
        look("delivered to a container agent");
        await snapshot("entries");
    });

    it("lists the entries without delivered figures, and counts them in the preview tile", async () => {
        fakeModel({ ordinary: [entry("o1"), entry("o2")], system: [] });
        mount();
        await flush();
        look("no figures");
        await snapshot("no-figures");
    });

    it("sorts the largest first when asked, and stops offering drag", async () => {
        const m = fakeModel({ system: [entry("s1", { is_system: true })], ordinary: [entry("o2"), entry("o1")], tokens: TOKENS, delivered: true, totals: { total: 500, count: 3 } });
        mount();
        await flush();
        look("in injection order");
        m.setLargest(true);
        await flush();
        look("largest first");
        await snapshot("largest-first");
    });

    it("shows the read-only CLAUDE.md tile, with and without a file", async () => {
        const m = fakeModel({ claude: { exists: true, content: "# rules", path: "/cfg/CLAUDE.md" } });
        mount();
        await flush();
        look("with a file");
        m.setClaude({ exists: false, content: null, path: "/cfg/CLAUDE.md" });
        await flush();
        look("without a file");
        m.setClaude(null);
        await flush();
        look("none");
        await snapshot("claude-config");
    });

    it("lists what was removed, restores an entry and reloads after an import", async () => {
        const m = fakeModel({ removed: [entry("x1", { removed_by: "operator", removed_at: 50 }), entry("x2", { removed_by: "agent:Naki", removed_at: 60 })], error: "Could not load" });
        mount();
        await flush();
        look("removed and an error");
        fireEvent.click(host.querySelectorAll(".global-bundle-removed-row button")[1]);
        await flush();
        fireEvent.click(host.querySelector('[data-stand="ImportBanner"] [data-cb]')!);
        await flush();
        m.setRemoved([]);
        m.setError(null);
        await flush();
        look("nothing removed");
        await snapshot("removed");
    });

    it("opens a full view from each kind of tile", async () => {
        const m = fakeModel({
            system: [entry("s1", { is_system: true })],
            ordinary: [entry("o1")],
            claude: { exists: true, content: "x", path: "/cfg/CLAUDE.md" },
            totals: { total: 10, count: 2 },
        });
        mount();
        await flush();
        const kinds = ["s1", "o1"];
        for (const id of kinds) {
            await choose(tileOf(id));
            m.setView(null);
            await flush();
        }
        for (const t of tiles().slice(2)) {
            await choose(t);
            m.setView(null);
            await flush();
        }
        await snapshot("open-tiles");
    });

    it("drags an ordinary entry onto another to move it, and ignores the rest", async () => {
        fakeModel({ system: [entry("s1", { is_system: true })], ordinary: [entry("o1"), entry("o2"), entry("o3")] });
        mount();
        await flush();
        const dt = dataTransfer();
        look("before the drag");
        fireEvent.dragStart(tileOf("o1"), { dataTransfer: dt });
        await flush();
        look("dragging o1");
        fireEvent.dragOver(tileOf("o1"), { dataTransfer: dt });
        fireEvent.dragOver(tileOf("o2"), { dataTransfer: dt });
        await flush();
        look("over o2");
        fireEvent.dragLeave(tileOf("o3"));
        fireEvent.dragLeave(tileOf("o2"));
        await flush();
        look("left o2");
        fireEvent.dragOver(tileOf("o3"), { dataTransfer: dt });
        fireEvent.drop(tileOf("o3"), { dataTransfer: dt });
        await flush();
        look("dropped on o3");
        fireEvent.dragStart(tileOf("o2"), { dataTransfer: dt });
        fireEvent.dragEnd(tileOf("o2"));
        await flush();
        look("drag ended");
        fireEvent.drop(tileOf("o1"), { dataTransfer: { ...dt, getData: () => "o3" } });
        fireEvent.drop(tileOf("o1"), { dataTransfer: { ...dt, getData: () => "o1" } });
        fireEvent.drop(tileOf("o1"), { dataTransfer: { ...dt, getData: () => "" } });
        await flush();
        await snapshot("drag");
    });

    it("shows a full view with its crumb and error, and goes back without asking when nothing changed", async () => {
        const m = fakeModel({ ordinary: [entry("o1")], view: { kind: "new" }, error: "Save failed" });
        mount();
        await flush();
        look("new");
        for (const [view, extra] of [
            [{ kind: "claude-config" }, {}],
            [{ kind: "preview" }, {}],
            [{ kind: "entry", id: "o1" }, { openEntry: entry("o1", { name: "Opened one" }) }],
        ] as const) {
            m.setView(view);
            m.setOpenEntry((extra as any).openEntry ?? null);
            await flush();
            look(`view ${view.kind}`);
        }
        m.setView({ kind: "entry", id: "gone" });
        m.setOpenEntry(null);
        m.setError(null);
        await flush();
        look("an entry that is not there, no error");
        fireEvent.click([...host.querySelectorAll("button")].find((b) => b.textContent?.includes("All global memory"))!);
        await flush();
        look("back to the tiles");
        await snapshot("full-view");
    });

    it("builds the tiles once and shows the same ones again each time a full view is closed", async () => {
        const m = fakeModel({ ordinary: [entry("o1")] });
        mount();
        await flush();
        await choose(tileOf("o1"));
        look("out to the entry");
        m.setView(null);
        await flush();
        await choose(tileOf("o1"));
        m.setView({ kind: "preview" });
        await flush();
        m.setView(null);
        await flush();
        look("back again");
        await snapshot("remount");
    });

    it("asks before leaving a full view with unsaved edits, and stays when told to keep editing", async () => {
        const m = fakeModel({ ordinary: [entry("o1")], view: { kind: "entry", id: "o1" }, openEntry: entry("o1"), dirty: true });
        mount();
        await flush();
        const back = () => fireEvent.click([...host.querySelectorAll("button")].find((b) => b.textContent?.includes("All global memory"))!);
        h.discard = false;
        back();
        await flush();
        look("kept editing");
        h.discard = true;
        back();
        await flush();
        look("discarded");
        m.setDirty(false);
        await snapshot("discard");
    });

    it("disposes its model when it goes away", async () => {
        fakeModel();
        const view = mount();
        await flush();
        view.unmount();
        await snapshot("dispose");
    });
});
