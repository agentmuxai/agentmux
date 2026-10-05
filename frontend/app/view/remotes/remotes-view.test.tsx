// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { RemoteRecord } from "@/app/store/rpc-api/remotes";

const rpc = vi.hoisted(() => ({
    RemotesListCommand: vi.fn(),
    RemoteSetConfigCommand: vi.fn(),
    RemoteForgetCommand: vi.fn(),
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
        helper: { state: "absent", version: "" },
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
                helper: { state: "installed", version: "0.59.9" },
                sessions: 2,
                agents: ["korp"],
            }),
            remote("me@box", { sources: ["recent"], last_used_ms: 5 }),
            remote("Ubuntu", { kind: "wsl", sources: ["wsl"], helper: { state: "none", version: "" } }),
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

    it("shows a failed action instead of dropping it", async () => {
        await renderWith([remote("db1")]);
        rpc.RemoteSetConfigCommand.mockRejectedValue(new Error("invalid setting for db1"));
        fireEvent.click(document.querySelector('[data-remote="db1"]')!);
        fireEvent.click(screen.getByRole("button", { name: "Hide" }));
        expect(await screen.findByText("Hide failed: invalid setting for db1")).toBeTruthy();
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
