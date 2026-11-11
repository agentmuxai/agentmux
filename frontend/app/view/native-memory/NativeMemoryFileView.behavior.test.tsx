// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Memory → Personal's full view of one file (`NativeMemoryFileView`), pinned as a whole ahead of splitting it into a
 * state file and view parts: each scenario mounts the real view over the real history and draft models and the real
 * kit, with stand-ins for the memory commands (whose answers the scenario controls), the Markdown renderer and the
 * hash (the same digest, taken on this thread), on a fake clock with the locale's date format stubbed, and logs the
 * page as an indented tree and, in order, every call made (each command with its request, each dirtiness report to
 * the manager). The logs are snapshots recorded on the unsplit view; the split must leave them byte for byte.
 * (NativeMemoryFileView.test.tsx asserts what the person is told; this pins the markup and the calls.)
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => {
    const T: string[] = [];
    const log = (line: string): void => {
        T.push(line);
    };
    const state = {
        saved: "v1" as string | null,
        versions: [] as any[],
        readError: null as string | null,
        historyError: null as string | null,
        write: null as null | (() => Promise<unknown>),
        diff: null as null | (() => Promise<{ diff: string }>),
        revert: null as null | (() => Promise<unknown>),
    };
    return { T, log, state };
});
const { T, log } = h;

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        NativeMemoryReadFileCommand: (_c: unknown, req: unknown) => {
            log(`NativeMemoryReadFileCommand ${JSON.stringify(req)}`);
            if (h.state.readError) return Promise.reject(new Error(h.state.readError));
            if (h.state.saved === null) return Promise.reject(new Error("agent:memory:read_file: not found"));
            return Promise.resolve({ content: h.state.saved });
        },
        NativeMemoryWriteFileCommand: (_c: unknown, req: any) => {
            log(`NativeMemoryWriteFileCommand ${JSON.stringify(req)}`);
            const pending = h.state.write;
            if (pending) return pending();
            h.state.saved = req.content;
            return Promise.resolve(null);
        },
        NativeMemoryHistoryCommand: (_c: unknown, req: unknown) => {
            log(`NativeMemoryHistoryCommand ${JSON.stringify(req)}`);
            if (h.state.historyError) return Promise.reject(new Error(h.state.historyError));
            return Promise.resolve({ versions: h.state.versions });
        },
        NativeMemoryDiffCommand: (_c: unknown, req: unknown) => {
            log(`NativeMemoryDiffCommand ${JSON.stringify(req)}`);
            return h.state.diff ? h.state.diff() : Promise.resolve({ diff: "-old line\n+new line\n same" });
        },
        NativeMemoryRevertCommand: (_c: unknown, req: unknown) => {
            log(`NativeMemoryRevertCommand ${JSON.stringify(req)}`);
            if (h.state.revert) return h.state.revert();
            h.state.saved = "reverted content";
            return Promise.resolve(null);
        },
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
// The same digest, taken on this thread: the real one finishes on another, which a fake clock can't wait for.
vi.mock("@/util/sha256", async () => {
    const { createHash } = await import("node:crypto");
    return { sha256Hex: (s: string) => Promise.resolve(createHash("sha256").update(s).digest("hex")) };
});
vi.mock("@/app/element/markdown", () => ({
    Markdown: (props: any) => (
        <div data-markdown="" data-scrollable={String(props.scrollable)}>
            {props.text}
        </div>
    ),
}));

import { ModalLayer } from "@/app/element/ModalLayer";
import { createConfirm } from "@/app/element/create-confirm";
import { NativeMemoryFileView } from "./NativeMemoryFileView";

const SNAP = (name: string) => `./__snapshots__/NativeMemoryFileView.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const ATTRS = /^(class|role|aria-.*|data-.*|title|style|disabled|type|id|checked|placeholder|spellcheck)$/;

/** A subtree as an indented tree: tags, classes, the attributes that carry state, and text. */
function dump(node: Node, depth = 0, out: string[] = []): string[] {
    const pad = "  ".repeat(depth);
    if (node.nodeType === Node.TEXT_NODE) {
        const t = (node.textContent ?? "").replace(/\s+/g, " ").trim();
        if (t) out.push(`${pad}"${t}"`);
        return out;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return out;
    const el = node as HTMLElement;
    const attrs = [...el.attributes]
        .filter((a) => ATTRS.test(a.name))
        .map((a) => `${a.name}=${JSON.stringify(a.value)}`);
    const value =
        el instanceof HTMLTextAreaElement
            ? ` value=${JSON.stringify(el.value)}`
            : el instanceof HTMLInputElement
              ? ` checked=${el.checked}`
              : "";
    out.push(`${pad}<${el.tagName.toLowerCase()}${attrs.length ? " " + attrs.join(" ") : ""}${value}>`);
    el.childNodes.forEach((c) => dump(c, depth + 1, out));
    return out;
}

const NOW = Date.parse("2026-11-10T12:00:00Z");
const DAY = 86_400_000;
const ver = (id: string, source: string, ageDays: number, tokens: number, extra: Record<string, unknown> = {}) => ({
    id,
    content_hash: `hash-${id}`,
    parent_version_id: null,
    source,
    source_detail: "",
    session_id: "",
    created_at: NOW - ageDays * DAY,
    size_bytes: tokens * 4,
    tokens_est: tokens,
    ...extra,
});
const VERSIONS = [
    ver("v5", "human", 0.1, 1200),
    ver("v4", "agent_inferred", 2, 1100),
    ver("v3", "jekt", 9, 900, { source_detail: JSON.stringify({ TIER: "coord", TRUST: "lan-verified" }) }),
    ver("v2", "external_fs_write", 20, 700, { source_detail: JSON.stringify({ via: "editor" }) }),
    ver("v1", "revert", 40, 300),
];

const [nonce, setNonce] = createSignal(0);
const bump = () => {
    log("-- refreshNonce bumped");
    setNonce((n) => n + 1);
};
let unmountView: (() => void) | null = null;

const open = (label: string, setup?: () => void) => {
    unmountView?.();
    cleanup();
    log(`-- open: ${label}`);
    h.state.saved = "v1";
    h.state.versions = VERSIONS;
    h.state.readError = null;
    h.state.historyError = null;
    h.state.write = null;
    h.state.diff = null;
    h.state.revert = null;
    setup?.();
    const confirm = createConfirm();
    const r = render(() => (
        <ModalLayer scope="pane">
            <confirm.Dialog />
            <NativeMemoryFileView
                agentId="a1"
                filename="MEMORY.md"
                refreshNonce={nonce}
                onDirtyChange={(d) => log(`onDirtyChange ${d}`)}
                confirm={confirm}
            />
        </ModalLayer>
    ));
    unmountView = r.unmount;
};
const look = (label: string) => {
    log(`-- ${label}`);
    log(dump(document.body).join("\n"));
};
const settle = async (ms = 0) => {
    await vi.advanceTimersByTimeAsync(ms);
    for (let i = 0; i < 20; i++) await Promise.resolve();
};
const button = (label: string, nth = 0) =>
    [...document.querySelectorAll<HTMLElement>("button")].filter((b) => b.textContent?.trim() === label)[nth];
const press = async (label: string, nth = 0) => {
    log(`-- press ${label}`);
    fireEvent.click(button(label, nth));
    await settle();
};
const textarea = () => document.querySelector<HTMLTextAreaElement>(".memory-content-textarea")!;
const type = async (value: string) => {
    log(`-- type ${JSON.stringify(value)}`);
    fireEvent.input(textarea(), { target: { value } });
    await settle();
};
const key = async (init: KeyboardEventInit, label: string) => {
    log(`-- key ${label}`);
    fireEvent.keyDown(textarea(), init);
    await settle();
};
const edit = async () => {
    await press("Edit");
};
const deferred = <T,>() => {
    let resolve!: (v: T) => void;
    let reject!: (e: unknown) => void;
    const promise = new Promise<T>((res, rej) => {
        resolve = res;
        reject = rej;
    });
    return { promise, resolve, reject };
};

beforeEach(() => {
    T.length = 0;
    vi.useFakeTimers({ now: NOW });
    vi.spyOn(Date.prototype, "toLocaleString").mockImplementation(function (this: Date) {
        return `T${this.getTime()}`;
    });
    localStorage.clear();
});
afterEach(() => {
    unmountView?.();
    unmountView = null;
    cleanup();
    vi.useRealTimers();
    vi.restoreAllMocks();
});

describe("NativeMemoryFileView, pinned as a whole", () => {
    it("reads a file: loading, its history, the content, a comparison, and what failed", async () => {
        open("loading, then loaded");
        look("before the reads answer");
        await settle();
        look("loaded");
        log("-- select two versions");
        fireEvent.click(document.querySelectorAll<HTMLInputElement>(".native-memory-history-item input")[3]);
        fireEvent.click(document.querySelectorAll<HTMLInputElement>(".native-memory-history-item input")[0]);
        await settle();
        look("diff shown");
        await press("Clear selection");
        look("selection cleared");
        const pending = deferred<{ diff: string }>();
        open("diff in flight", () => {
            h.state.diff = () => pending.promise;
        });
        await settle();
        fireEvent.click(document.querySelectorAll<HTMLInputElement>(".native-memory-history-item input")[1]);
        fireEvent.click(document.querySelectorAll<HTMLInputElement>(".native-memory-history-item input")[2]);
        await settle();
        look("loading diff");
        pending.resolve({ diff: "+added" });
        await settle();
        look("diff answered");
        open("file not found", () => {
            h.state.saved = null;
        });
        await settle();
        look("not found: no content, Edit disabled");
        open("read fails", () => {
            h.state.readError = "boom";
        });
        await settle();
        look("read failed");
        open("history fails", () => {
            h.state.historyError = "history down";
        });
        await settle();
        look("history failed");
        open("empty file, no versions", () => {
            h.state.saved = "";
            h.state.versions = [];
        });
        await settle();
        look("empty");
        await snapshot("read");
    });

    it("edits a file: save, cancel, the discard question, keys and a failed save", async () => {
        open("edit and save");
        await settle();
        await edit();
        look("editing");
        await type("v2");
        look("typed v2");
        const pending = deferred<unknown>();
        h.state.write = () => pending.promise;
        await press("Save");
        look("saving");
        h.state.write = null;
        h.state.saved = "v2";
        pending.resolve(null);
        await settle();
        look("saved");

        await edit();
        look("editing again, nothing typed");
        await press("Cancel");
        look("cancelled with no changes");

        await edit();
        await type("dirty");
        await press("Cancel");
        look("asked to discard");
        await press("Keep editing");
        look("kept editing");
        await press("Cancel");
        await press("Discard");
        look("discarded");

        await edit();
        await type("with keys");
        await key({ key: "Escape" }, "Escape on a dirty draft");
        look("asked to discard (Escape)");
        await press("Discard");
        look("discarded (Escape)");
        await edit();
        await key({ key: "Escape" }, "Escape on a clean draft");
        look("closed by Escape");
        await edit();
        await type("saved by Ctrl+S");
        await key({ key: "s", ctrlKey: true }, "Ctrl+S");
        look("saved by Ctrl+S");
        await edit();
        await type("saved by Cmd+S");
        await key({ key: "S", metaKey: true }, "Cmd+S");
        await edit();
        await type("not a save");
        await key({ key: "s", ctrlKey: true, shiftKey: true }, "Ctrl+Shift+S");
        look("Ctrl+Shift+S does nothing");

        open("save fails", () => {
            h.state.write = () => Promise.reject(new Error("disk full"));
        });
        await settle();
        await edit();
        await type("lost?");
        await press("Save");
        look("save failed, draft kept");
        await snapshot("edit");
    });

    it("follows live changes without wiping a draft, and shows a conflict", async () => {
        open("not editing");
        await settle();
        h.state.saved = "fresh";
        bump();
        await settle();
        look("refreshed in place");

        open("editing, no changes yet");
        await settle();
        await edit();
        h.state.saved = "updated underneath";
        bump();
        await settle();
        look("followed along");

        open("dirty, file changed");
        await settle();
        await edit();
        await type("my draft");
        h.state.saved = "the agent wrote this";
        bump();
        await settle();
        look("banner shown");
        await press("View change");
        look("change viewed");
        await press("Keep editing", 0);
        look("kept editing from the banner");

        open("dirty, file unchanged");
        await settle();
        await edit();
        await type("my draft");
        bump();
        await settle();
        look("silent");

        open("dirty, file deleted");
        await settle();
        await edit();
        await type("my draft");
        h.state.saved = null;
        bump();
        await settle();
        look("banner: deleted");
        await press("Discard my edits");
        look("edits discarded");

        open("save refused as a conflict");
        await settle();
        await edit();
        await type("my draft");
        h.state.saved = "someone else";
        h.state.write = () => Promise.reject(new Error("agent:memory:write_file: conflict: MEMORY.md changed since your edit began"));
        await press("Save");
        look("refused");
        h.state.write = null;
        await press("Save anyway");
        look("overwritten");
        await snapshot("live");
    });

    it("reverts to an earlier version, with and without an open draft", async () => {
        open("revert while reading");
        await settle();
        await press("Revert to this", 0);
        look("asked to confirm");
        await press("Cancel", 0);
        look("revert cancelled");
        await press("Revert to this", 0);
        const pending = deferred<unknown>();
        h.state.revert = () => pending.promise;
        await press("Confirm revert");
        look("reverting");
        h.state.revert = null;
        h.state.saved = "reverted content";
        h.state.versions = [ver("v6", "revert", 0, 1000), ...VERSIONS];
        pending.resolve(null);
        await settle();
        look("reverted");

        open("revert while editing");
        await settle();
        await edit();
        await press("Revert to this", 1);
        look("revert asked, Save disabled while saving is off");
        await press("Confirm revert");
        look("reverted with an open editor");

        open("revert fails", () => {
            h.state.revert = () => Promise.reject(new Error("no such version"));
        });
        await settle();
        await press("Revert to this", 0);
        await press("Confirm revert");
        look("revert failed");
        await snapshot("revert");
    });
});
