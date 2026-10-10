// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { RemoteRecord } from "@/app/store/rpc-api/remotes";

const rpc = vi.hoisted(() => ({
    RemotesListCommand: vi.fn(),
    RemoteSetConfigCommand: vi.fn(),
    RemoteForgetCommand: vi.fn(),
    RemoteHelperRemoveCommand: vi.fn(),
    RemoteAddCommand: vi.fn(),
    RemoteTestCommand: vi.fn(),
    RemoteSshLocateCommand: vi.fn(),
    RemoteAgentRevokeCommand: vi.fn(),
    ConnConnectCommand: vi.fn(),
    ConnDisconnectCommand: vi.fn(),
}));
const tabRpcCall = vi.hoisted(() => vi.fn());

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: rpc }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: { rpcCall: tabRpcCall } }));
vi.mock("@/app/store/mps", () => ({ muxEventSubscribe: vi.fn(() => () => {}) }));
vi.mock("@/app/view/term/hostSessions", () => ({ showHostSessions: vi.fn() }));

import { RemotesViewModel } from "./remotes-model";
import { RemotesView, rowMenu } from "./remotes-view";

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

async function renderWith(records: RemoteRecord[]) {
    rpc.RemotesListCommand.mockResolvedValue(records);
    const model = new RemotesViewModel({ blockId: "blk-1" }, { subscribe: false });
    await model.refresh();
    const result = render(() => <RemotesView model={model} />);
    return { ...result, model };
}

describe("RemotesView", () => {
    beforeEach(() => {
        Object.values(rpc).forEach((f) => f.mockReset().mockResolvedValue(undefined));
        tabRpcCall.mockReset().mockResolvedValue(undefined);
    });
    afterEach(() => cleanup());

    it("shows each remote once, in its section, with platform, helper and activity", async () => {
        await renderWith([
            remote("db1", {
                settings: { "display:name": "prod-db", "display:color": "#e5484d" },
                status: { state: "connected", error: "" },
                platform: { os: "linux", arch: "x86_64" },
                helper: { state: "installed", version: "0.59.9", installed: true },
                sessions: 2,
                agents: ["korp"],
            }),
            remote("me@box", { sources: ["recent"], last_used_ms: 5 }),
            remote("Ubuntu", { kind: "wsl", sources: ["wsl"], helper: { state: "none", version: "", installed: false } }),
        ]);
        const headings = Array.from(document.querySelectorAll(".remotes-section-heading")).map((h) => h.textContent);
        expect(headings).toEqual(["SSH hosts", "Recent", "WSL"]);

        const row = document.querySelector('[data-remote="db1"]')!;
        expect(row.textContent).toContain("prod-db");
        expect(row.querySelector(".remotes-realname")?.textContent).toBe("db1");
        expect(row.textContent).toContain("Linux · x86_64");
        expect(row.textContent).toContain("Helper 0.59.9");
        expect(row.textContent).toContain("2 sessions · 1 agent");
        expect(row.querySelector(".remotes-dot")?.classList.contains("is-connected")).toBe(true);
        expect((row.querySelector(".remotes-swatch") as HTMLElement).style.background).not.toBe("");
    });

    it("keeps Hidden collapsed until opened", async () => {
        await renderWith([remote("gone", { settings: { "display:hidden": true } })]);
        expect(document.querySelector('[data-remote="gone"]')).toBeNull();
        fireEvent.click(screen.getByRole("button", { name: /Hidden \(1\)/ }));
        expect(document.querySelector('[data-remote="gone"]')).not.toBeNull();
    });

    it("says so when there are no remotes, or none match", async () => {
        const { model } = await renderWith([]);
        expect(screen.getByText(/No remotes yet/)).toBeTruthy();
        model.setFilter("x");
        expect(screen.getByText(/No remotes match/)).toBeTruthy();
    });

    it("opens a terminal and Hangar beside this pane", async () => {
        await renderWith([remote("db1")]);
        fireEvent.click(screen.getByRole("button", { name: "New terminal on db1" }));
        fireEvent.click(screen.getByRole("button", { name: "Browse files on db1" }));
        await Promise.resolve();
        expect(tabRpcCall).toHaveBeenCalledWith(
            "pane.open",
            { view: "term", connection: "db1", split_direction: "right", split_reference_block_id: "blk-1" },
            {}
        );
        expect(tabRpcCall).toHaveBeenCalledWith(
            "pane.open",
            expect.objectContaining({ view: "files", cwd: "~", connection: "db1" }),
            {}
        );
    });

    it("expands a row and writes its settings", async () => {
        await renderWith([remote("db1")]);
        fireEvent.click(document.querySelector('[data-remote="db1"]')!);
        fireEvent.click(screen.getByRole("button", { name: "Pin" }));
        fireEvent.click(screen.getByRole("radio", { name: "Blue" }));
        const nick = screen.getByPlaceholderText("db1") as HTMLInputElement;
        nick.value = "prod-db";
        fireEvent.blur(nick);
        await Promise.resolve();
        expect(rpc.RemoteSetConfigCommand).toHaveBeenCalledWith(expect.anything(), {
            connection: "db1",
            values: { "display:pinned": true },
        });
        expect(rpc.RemoteSetConfigCommand).toHaveBeenCalledWith(expect.anything(), {
            connection: "db1",
            values: { "display:color": "#0090ff" },
        });
        expect(rpc.RemoteSetConfigCommand).toHaveBeenCalledWith(expect.anything(), {
            connection: "db1",
            values: { "display:name": "prod-db" },
        });
    });

    it("leaves Enter on a quick button to the button, not the row", async () => {
        const { model } = await renderWith([remote("db1")]);
        const button = screen.getByRole("button", { name: "New terminal on db1" });
        const ev = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });
        button.dispatchEvent(ev);
        expect(ev.defaultPrevented).toBe(false);
        expect(model.expanded()).toBeNull();
        fireEvent.keyDown(document.querySelector('[data-remote="db1"]')!, { key: "Enter" });
        expect(model.expanded()).toBe("db1");
    });

    it("commits a nickname once when Enter is pressed", async () => {
        await renderWith([remote("db1")]);
        fireEvent.click(document.querySelector('[data-remote="db1"]')!);
        const nick = screen.getByPlaceholderText("db1") as HTMLInputElement;
        nick.focus();
        nick.value = "prod-db";
        fireEvent.keyDown(nick, { key: "Enter" });
        await Promise.resolve();
        expect(rpc.RemoteSetConfigCommand).toHaveBeenCalledTimes(1);
    });

    it("shows a failed action instead of dropping it", async () => {
        await renderWith([remote("db1")]);
        rpc.RemoteSetConfigCommand.mockRejectedValue(new Error("invalid setting for db1"));
        fireEvent.click(document.querySelector('[data-remote="db1"]')!);
        fireEvent.click(screen.getByRole("button", { name: "Hide" }));
        expect(await screen.findByText("Hide failed: invalid setting for db1")).toBeTruthy();
    });

    it("sets the host's helper install answer, and Ask clears it", async () => {
        await renderWith([remote("db1", { settings: { "conn:helper": "never" } })]);
        fireEvent.click(document.querySelector('[data-remote="db1"]')!);
        const select = screen.getByLabelText("Install the helper") as HTMLSelectElement;
        expect(select.value).toBe("never");
        fireEvent.change(select, { target: { value: "ask" } });
        await Promise.resolve();
        expect(rpc.RemoteSetConfigCommand).toHaveBeenCalledWith(expect.anything(), {
            connection: "db1",
            values: { "conn:helper": null },
        });
    });

    it("removes the helper through srv, which asks the user; Keep It is not an error", async () => {
        const { model } = await renderWith([remote("db1", { helper: { state: "installed", version: "0.59.9", installed: true } })]);
        fireEvent.click(document.querySelector('[data-remote="db1"]')!);
        rpc.RemoteHelperRemoveCommand.mockRejectedValueOnce(new Error("kept: the user chose not to remove it"));
        fireEvent.click(screen.getByRole("button", { name: "Remove helper" }));
        await new Promise((r) => setTimeout(r, 0));
        expect(rpc.RemoteHelperRemoveCommand).toHaveBeenCalledWith(
            expect.anything(),
            { connection: "db1", blockid: "blk-1" },
            expect.anything()
        );
        expect(model.notice()).toBe("");
    });

    it("offers Remove helper only where the helper is installed", async () => {
        await renderWith([remote("db1")]);
        fireEvent.click(document.querySelector('[data-remote="db1"]')!);
        expect(screen.queryByRole("button", { name: "Remove helper" })).toBeNull();
    });

    it("expands the host another pane asked for, once", async () => {
        rpc.RemotesListCommand.mockResolvedValue([remote("db1"), remote("web")]);
        const setMeta = vi.fn(async () => {});
        const model = new RemotesViewModel(
            { blockId: "blk-1", meta: () => ({ "remotes:expand": "web" }) as MetaType, setMeta },
            { subscribe: false }
        );
        await model.refresh();
        render(() => <RemotesView model={model} />);
        expect(model.expanded()).toBe("web");
        expect(setMeta).toHaveBeenCalledWith({ "remotes:expand": null });
    });

    it("adds just a destination to AgentMux's settings, named user@host:port, with nothing in ssh config", async () => {
        const { model } = await renderWith([]);
        fireEvent.click(screen.getByRole("button", { name: /Add remote/ }));
        const field = screen.getByLabelText("Connect to") as HTMLInputElement;
        expect(field.placeholder).toBe("user@host:port");
        fireEvent.input(field, { target: { value: "me@db1.example.com:2222" } });
        fireEvent.click(screen.getByRole("button", { name: "Add" }));
        await new Promise((r) => setTimeout(r, 0));
        expect(rpc.RemoteAddCommand).not.toHaveBeenCalled();
        expect(rpc.RemoteSetConfigCommand).toHaveBeenCalledWith(expect.anything(), {
            connection: "me@db1.example.com:2222",
            values: { "display:name": "me@db1.example.com:2222" },
        });
        expect(model.expanded()).toBe("me@db1.example.com:2222");
    });

    it("takes port 22 when none is typed", async () => {
        await renderWith([]);
        fireEvent.click(screen.getByRole("button", { name: /Add remote/ }));
        fireEvent.input(screen.getByLabelText("Connect to"), { target: { value: "asafe@127.0.0.1" } });
        fireEvent.click(screen.getByRole("button", { name: "Add" }));
        await new Promise((r) => setTimeout(r, 0));
        expect(rpc.RemoteSetConfigCommand).toHaveBeenCalledWith(expect.anything(), {
            connection: "asafe@127.0.0.1:22",
            values: { "display:name": "asafe@127.0.0.1:22" },
        });
    });

    it("writes ssh config, under a plain alias, only for an identity file or jump host from Advanced", async () => {
        const { model } = await renderWith([]);
        fireEvent.click(screen.getByRole("button", { name: /Add remote/ }));
        const field = (label: string) => screen.getByLabelText(label) as HTMLInputElement;
        fireEvent.input(field("Connect to"), { target: { value: "10.0.0.5" } });
        fireEvent.input(field("Name"), { target: { value: "lab" } });
        fireEvent.input(field("Identity file"), { target: { value: "~/.ssh/lab" } });
        fireEvent.input(field("Jump host"), { target: { value: "bastion" } });
        fireEvent.click(screen.getByRole("button", { name: "Add" }));
        await new Promise((r) => setTimeout(r, 0));
        expect(rpc.RemoteAddCommand).toHaveBeenCalledWith(
            expect.anything(),
            { alias: "10-0-0-5", hostname: "10.0.0.5", user: "", port: "", identityfile: "~/.ssh/lab", proxyjump: "bastion", blockid: "blk-1" },
            expect.anything()
        );
        expect(rpc.RemoteSetConfigCommand).toHaveBeenCalledWith(expect.anything(), {
            connection: "10-0-0-5",
            values: { "display:name": "lab" },
        });
        expect(model.expanded()).toBe("10-0-0-5");
    });

    it("says how to write the destination when it can't be read", async () => {
        await renderWith([]);
        fireEvent.click(screen.getByRole("button", { name: /Add remote/ }));
        fireEvent.input(screen.getByLabelText("Connect to"), { target: { value: "me@host:port" } });
        fireEvent.click(screen.getByRole("button", { name: "Add" }));
        await new Promise((r) => setTimeout(r, 0));
        expect(rpc.RemoteAddCommand).not.toHaveBeenCalled();
        expect(rpc.RemoteSetConfigCommand).not.toHaveBeenCalled();
        expect(document.querySelector(".remotes-add .remotes-detail-error")?.textContent).toContain("user@host:port");
    });

    it("keeps the form open, without an error, when the user cancels in the window", async () => {
        await renderWith([]);
        rpc.RemoteAddCommand.mockRejectedValueOnce(new Error("kept: the user chose not to add it"));
        fireEvent.click(screen.getByRole("button", { name: /Add remote/ }));
        fireEvent.input(screen.getByLabelText("Connect to"), { target: { value: "db1" } });
        fireEvent.input(screen.getByLabelText("Identity file"), { target: { value: "~/.ssh/db1" } });
        fireEvent.click(screen.getByRole("button", { name: "Add" }));
        await new Promise((r) => setTimeout(r, 0));
        expect(screen.getByLabelText("Connect to")).toBeTruthy();
        expect(rpc.RemoteSetConfigCommand).not.toHaveBeenCalled();
        expect(document.querySelector(".remotes-add .remotes-detail-error")).toBeNull();
    });

    it("tests a connection and says how it went", async () => {
        await renderWith([remote("db1")]);
        rpc.RemoteTestCommand.mockResolvedValueOnce({ ok: false, message: "Permission denied (publickey)." });
        fireEvent.click(document.querySelector('[data-remote="db1"]')!);
        fireEvent.click(screen.getByRole("button", { name: "Test connection" }));
        expect(await screen.findByText("Test failed: Permission denied (publickey).")).toBeTruthy();
        expect(rpc.RemoteTestCommand).toHaveBeenCalledWith(
            expect.anything(),
            { connection: "db1", blockid: "blk-1" },
            expect.anything()
        );
    });

    it("opens the ssh config file that defines the host, at its Host line", async () => {
        const { model } = await renderWith([remote("db1")]);
        rpc.RemoteSshLocateCommand.mockResolvedValueOnce({ path: "/home/u/.ssh/config", line: 12 });
        fireEvent.click(document.querySelector('[data-remote="db1"]')!);
        fireEvent.click(screen.getByRole("button", { name: "Edit in ssh config" }));
        await new Promise((r) => setTimeout(r, 0));
        expect(tabRpcCall).toHaveBeenCalledWith(
            "pane.open",
            expect.objectContaining({ view: "editor", file: "/home/u/.ssh/config", line: 12 }),
            {}
        );
        expect(model.notice()).toBeFalsy();
    });

    it("lists the agents allowed on a host, each with Revoke", async () => {
        await renderWith([remote("db1", { agents: ["agentx", "korp"] })]);
        fireEvent.click(document.querySelector('[data-remote="db1"]')!);
        expect([...document.querySelectorAll(".remotes-agent-name")].map((e) => e.textContent)).toEqual(["agentx", "korp"]);
        fireEvent.click(screen.getByRole("button", { name: "Revoke korp on db1" }));
        await Promise.resolve();
        expect(rpc.RemoteAgentRevokeCommand).toHaveBeenCalledWith(expect.anything(), { connection: "db1", agent: "korp" });
    });

    it("offers Forget only for a recent connection", async () => {
        const { model } = await renderWith([]);
        const labels = (r: RemoteRecord) => rowMenu(model, r).filter((i) => i.type === "action").map((i) => i.label);
        expect(labels(remote("me@box", { sources: ["recent"] }))).toContain("Forget");
        expect(labels(remote("db1"))).not.toContain("Forget");
        expect(labels(remote("db1", { status: { state: "connected", error: "" } }))).toContain("Disconnect");
        expect(labels(remote("Ubuntu", { kind: "wsl", sources: ["wsl"] }))).not.toContain("Sessions…");
    });
});
