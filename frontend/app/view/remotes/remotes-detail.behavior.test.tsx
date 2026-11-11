// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The details opened below a remote's card in the Remotes pane (`Detail` in remotes-view.tsx), pinned as a whole ahead
 * of splitting it into smaller views: each scenario mounts the real pane over the real view model and the real kit,
 * with stand-ins for srv's remote commands (whose answers the scenario controls), the pane opener and the host
 * sessions dialog, on a fake clock, opens a card, and logs the details as an indented tree and, in order, every call
 * made (each command with its request, the dialog, the refresh the model schedules, a failure shown as the pane's notice).
 * The logs are snapshots recorded on the unsplit pane; the split must leave them byte for byte.
 * (remotes-view.test.tsx asserts the pane against the same stand-ins; this pins the details' markup and calls.)
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { RemoteRecord } from "@/app/store/rpc-api/remotes";

const h = vi.hoisted(() => {
    const T: string[] = [];
    const log = (line: string): void => {
        T.push(line);
    };
    const state = { records: [] as any[], replies: {} as Record<string, any> };
    const command = (name: string) => (_client: unknown, ...args: unknown[]) => {
        log([name, ...args.map((a) => JSON.stringify(a))].join(" "));
        const reply = state.replies[name];
        if (name === "RemotesListCommand") return Promise.resolve(state.records);
        return typeof reply === "function" ? reply() : Promise.resolve(reply);
    };
    return { T, log, command, state };
});
const { T, log } = h;

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        RemotesListCommand: h.command("RemotesListCommand"),
        RemoteSetConfigCommand: h.command("RemoteSetConfigCommand"),
        RemoteForgetCommand: h.command("RemoteForgetCommand"),
        RemoteHelperRemoveCommand: h.command("RemoteHelperRemoveCommand"),
        RemoteAddCommand: h.command("RemoteAddCommand"),
        RemoteTestCommand: h.command("RemoteTestCommand"),
        RemoteSshLocateCommand: h.command("RemoteSshLocateCommand"),
        RemoteAgentRevokeCommand: h.command("RemoteAgentRevokeCommand"),
        ConnConnectCommand: h.command("ConnConnectCommand"),
        ConnDisconnectCommand: h.command("ConnDisconnectCommand"),
        PaneOpenCommand: h.command("PaneOpenCommand"),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ muxEventSubscribe: vi.fn(() => () => {}) }));
vi.mock("@/app/view/term/hostSessions", () => ({
    showHostSessions: (name: string, blockId: string) => (log(`showHostSessions ${name} ${blockId}`), Promise.resolve()),
}));

import { RemotesViewModel } from "./remotes-model";
import { RemotesView } from "./remotes-view";

const SNAP = (name: string) => `./__snapshots__/remotes-detail.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const ATTRS = /^(class|role|aria-.*|data-.*|title|style|disabled|type|id|placeholder)$/;

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
    const value = el instanceof HTMLInputElement || el instanceof HTMLSelectElement ? ` value=${JSON.stringify(el.value)}` : "";
    out.push(`${pad}<${el.tagName.toLowerCase()}${attrs.length ? " " + attrs.join(" ") : ""}${value}>`);
    el.childNodes.forEach((c) => dump(c, depth + 1, out));
    return out;
}

let host: HTMLElement;
let model: RemotesViewModel;
const look = (label: string) => {
    log(`-- ${label}`);
    const detail = host.querySelector(".remotes-detail");
    log(detail ? dump(detail).join("\n") : "no details");
    const notice = host.querySelector(".ui-notice");
    if (notice) log(`notice: ${notice.textContent}`);
};
const flush = async (ms = 0) => {
    vi.advanceTimersByTime(ms);
    for (let i = 0; i < 12; i++) await Promise.resolve();
};
const press = async (label: string | RegExp, nth = 0) => {
    const found = [...host.querySelectorAll<HTMLElement>(".remotes-detail button")].filter((b) =>
        typeof label === "string" ? (b.textContent?.trim() === label || b.getAttribute("aria-label") === label) : label.test(b.textContent ?? "") || label.test(b.getAttribute("aria-label") ?? "")
    );
    log(`-- press ${label}`);
    fireEvent.click(found[nth]);
    await flush();
};
const choose = async (nth: number, value: string) => {
    const select = host.querySelectorAll<HTMLSelectElement>(".remotes-detail select")[nth];
    log(`-- choose ${value}`);
    select.value = value;
    fireEvent.change(select);
    await flush();
};
const nickname = () => host.querySelector<HTMLInputElement>(".remotes-detail .remotes-setting input")!;

function remote(name: string, over: Partial<RemoteRecord> = {}): RemoteRecord {
    return {
        name,
        kind: "ssh",
        sources: ["ssh_config"],
        status: { state: "disconnected", error: "" },
        platform: null,
        helper: { state: "absent", version: "", installed: false },
        sessions: null,
        agents: [],
        settings: {},
        last_used_ms: null,
        ...over,
    };
}

async function open(records: RemoteRecord[], name = records[0].name) {
    h.state.records = records;
    model = new RemotesViewModel({ blockId: "blk-1" }, { subscribe: false });
    await model.refresh();
    const view = render(() => <RemotesView model={model} />, { container: document.body.appendChild(document.createElement("div")) });
    host = view.container;
    fireEvent.click(host.querySelector(`[data-remote="${name}"]`)!);
    await flush();
    T.length = 0;
}

beforeEach(() => {
    T.length = 0;
    h.state.records = [];
    h.state.replies = {};
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
});
afterEach(() => {
    model?.dispose();
    cleanup();
    vi.useRealTimers();
    document.body.innerHTML = "";
});

describe("the Remotes details, pinned", () => {
    it("shows a plain ssh remote's actions (Connect, Sessions, Pin, Hide, Test connection), its nickname, the colours, and the two ssh-only settings at their defaults", async () => {
        await open([remote("db1")]);
        look("a plain ssh remote");
        return snapshot("plain");
    });

    it("shows what each kind of remote has: an error, Disconnect, Unpin and Unhide, Forget for a recent one, Remove helper, Edit in ssh config, the agents allowed (each with Revoke), a nickname, a colour and the stored settings; none of the ssh actions for WSL; and no connection action while a login is under way", async () => {
        const all = remote("db1", {
            sources: ["ssh_config", "recent"],
            status: { state: "connected", error: "" },
            helper: { state: "installed", version: "0.59.9", installed: true },
            agents: ["korp", "clamk"],
            settings: {
                "display:name": "prod-db",
                "display:color": "#0090ff",
                "display:pinned": true,
                "term:durable": true,
                "conn:helper": "always",
            },
        });
        await open([all, remote("web"), remote("login"), remote("Ubuntu"), remote("gone")]);
        look("everything");
        h.state.records = [
            { ...all, status: { state: "error", error: "Connection refused" }, settings: { "display:hidden": true, "term:durable": false, "conn:helper": "never" } },
            remote("web"),
            remote("login"),
            remote("Ubuntu"),
            remote("gone"),
        ];
        await model.refresh();
        await flush();
        look("an error, hidden, durable off, helper never");
        fireEvent.click(host.querySelector('[data-remote="login"]')!);
        h.state.records = [remote("login", { status: { state: "connecting", error: "" } })];
        await model.refresh();
        fireEvent.click(host.querySelector('[data-remote="login"]')!);
        await flush();
        look("a login under way");
        h.state.records = [remote("Ubuntu", { kind: "wsl", sources: ["wsl"] })];
        await model.refresh();
        fireEvent.click(host.querySelector('[data-remote="Ubuntu"]')!);
        await flush();
        look("WSL");
        return snapshot("kinds");
    });

    it("runs each action: Connect, Sessions, Pin, Hide, Forget and Remove helper (a kept helper is not a failure), and shows a failed one as the pane's notice", async () => {
        await open([remote("db1", { sources: ["ssh_config", "recent"], helper: { state: "installed", version: "1", installed: true } })]);
        await press("Connect");
        await press("Sessions…");
        await flush(5000);
        await press("Pin");
        await press("Hide");
        await press("Remove helper");
        h.state.replies.RemoteHelperRemoveCommand = () => Promise.reject(new Error("kept: sessions still open"));
        await press("Remove helper");
        h.state.replies.RemoteHelperRemoveCommand = () => Promise.reject(new Error("ssh refused"));
        await press("Remove helper");
        look("a failed removal");
        h.state.replies.ConnConnectCommand = () => Promise.reject("plain refusal");
        await press("Connect");
        look("a failed connect");
        await press("Forget");
        look("forgotten");
        return snapshot("actions");
    });

    it("tests the connection (off and 'Testing…' while it runs, then the result: connected, the failure's message, or 'ssh failed'), opens the ssh config at the host's line, and says when the config does not define it; and disconnects a connected remote", async () => {
        await open([remote("db1")]);
        let finish!: (v: unknown) => void;
        h.state.replies.RemoteTestCommand = () => new Promise((res) => (finish = res));
        await press("Test connection");
        look("testing");
        await press("Testing…");
        finish({ ok: true });
        await flush();
        look("test passed");
        h.state.replies.RemoteTestCommand = async () => ({ ok: false, message: "permission denied" });
        await press("Test connection");
        look("test failed");
        h.state.replies.RemoteTestCommand = async () => ({ ok: false, message: "" });
        await press("Test connection");
        look("test failed with no message");
        h.state.replies.RemoteSshLocateCommand = async () => ({ path: "/home/u/.ssh/config", line: 12 });
        await press("Edit in ssh config");
        h.state.replies.RemoteSshLocateCommand = async () => null;
        await press("Edit in ssh config");
        look("not defined in the config");
        h.state.records = [remote("db1", { status: { state: "connected", error: "" } })];
        await model.refresh();
        await flush();
        await press("Disconnect");
        return snapshot("test");
    });

    it("saves the settings: a nickname on blur or Enter (not when unchanged, a blank one clears it), a colour or none, keep-sessions-alive on, off or as in Settings, and the helper policy always, never or as in Settings; and revokes an agent", async () => {
        await open([remote("db1", { agents: ["korp"], settings: { "display:name": "prod" } })]);
        nickname().value = "prod";
        fireEvent.blur(nickname());
        await flush();
        log("-- blurred unchanged");
        nickname().value = "  staging ";
        fireEvent.blur(nickname());
        await flush();
        nickname().value = "";
        fireEvent.blur(nickname());
        await flush();
        nickname().value = "via enter";
        const blur = vi.spyOn(nickname(), "blur");
        fireEvent.keyDown(nickname(), { key: "Enter" });
        log(`-- Enter blurs: ${blur.mock.calls.length}`);
        fireEvent.keyDown(nickname(), { key: "a" });
        log(`-- another key blurs: ${blur.mock.calls.length}`);
        await press("Red");
        await press("None");
        await choose(0, "on");
        await choose(0, "off");
        await choose(0, "default");
        await choose(1, "always");
        await choose(1, "never");
        await choose(1, "ask");
        await press("Revoke korp on db1");
        await flush(5000);
        look("settled");
        return snapshot("settings");
    });
});
