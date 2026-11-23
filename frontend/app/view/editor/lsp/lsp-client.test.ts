// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Editor's language server client when a running server is no longer one the host may serve: srv refuses a
 * file that is a container agent's, and tells every client when it stops a server whose folder became one.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

const start = vi.fn();
const send = vi.fn();
const stop = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        LspStartCommand: (...a: unknown[]) => start(...a),
        LspSendCommand: (...a: unknown[]) => send(...a),
        LspStopCommand: (...a: unknown[]) => stop(...a),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
let hear: ((envelope: unknown) => void) | null = null;
vi.mock("@/app/store/mps", () => ({
    onMuxEvent: (_: string, f: (envelope: unknown) => void) => {
        hear = f;
        return () => {
            hear = null;
        };
    },
}));

import { LspClient, NOT_A_FILE_ERROR, NOT_ON_HOST_ERROR, NOT_ON_HOST_NOTICE } from "./lsp-client";

type Sent = { method?: string; id?: number; params?: { textDocument?: { uri?: string } } };

/** A server that answers `initialize`, and refuses what `refuse` gives a reason for. */
function aServer(refuse: (message: Sent) => string | null = () => null) {
    start.mockResolvedValue({ server_id: "srv-1", workspace_root: "/proj" });
    stop.mockResolvedValue(undefined);
    send.mockImplementation(async (_client: unknown, req: { message: Sent }) => {
        const why = refuse(req.message);
        if (why) throw new Error(why);
        if (req.message.method === "initialize") {
            queueMicrotask(() => hear?.({ server_id: "srv-1", message: { id: req.message.id, result: { capabilities: {} } } }));
        }
    });
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

async function aReadyClient() {
    const client = new LspClient("typescript", "/proj/src/a.ts");
    expect(await client.start()).toBe(true);
    expect(client.getState().kind).toBe("ready");
    return client;
}

describe("a running language server that may no longer serve a file", () => {
    beforeEach(() => {
        start.mockReset();
        send.mockReset();
        stop.mockReset();
        hear = null;
    });

    it("keeps serving a plain file of the project", async () => {
        aServer();
        const client = await aReadyClient();
        await client.didOpen("/proj/src/b.ts", "", "typescript");
        await settle();
        expect(client.getState().kind).toBe("ready");
    });

    it("shows the folder as a container agent's when srv refuses a file it was asked to open", async () => {
        aServer((m) => (m.method === "textDocument/didOpen" && m.params?.textDocument?.uri?.endsWith("/linked.ts") ? NOT_ON_HOST_ERROR : null));
        const client = await aReadyClient();
        await client.didOpen("/proj/src/linked.ts", "", "typescript");
        await settle();
        expect(client.getState()).toEqual({ kind: "notOnHost" });
    });

    it("still shows any other failed send as a crash", async () => {
        aServer((m) => (m.method === "textDocument/didChange" ? "the pipe is closed" : null));
        const client = await aReadyClient();
        await client.didChange("/proj/src/a.ts", "x");
        await settle();
        expect(client.getState()).toEqual({ kind: "crashed", error: "the pipe is closed" });
    });

    it("moves to the container agent's folder when srv stops its server, and ignores that for another server", async () => {
        aServer();
        const client = await aReadyClient();
        const notice = { jsonrpc: "2.0", method: NOT_ON_HOST_NOTICE, params: {} };
        hear?.({ server_id: "srv-other", message: notice });
        expect(client.getState().kind).toBe("ready");
        hear?.({ server_id: "srv-1", message: notice });
        expect(client.getState()).toEqual({ kind: "notOnHost" });
    });

    it("sends a server only the messages srv checks: none of them gives it a folder", async () => {
        aServer();
        const client = await aReadyClient();
        await client.didOpen("/proj/src/a.ts", "", "typescript");
        await client.didChange("/proj/src/a.ts", "x");
        await client.didClose("/proj/src/a.ts");
        await client.dispose();
        const methods = send.mock.calls.map((call) => (call[1] as { message: Sent }).message.method);
        expect(new Set(methods)).toEqual(
            new Set(["initialize", "initialized", "textDocument/didOpen", "textDocument/didChange", "textDocument/didClose"]),
        );
    });

    it("sends nothing more to a server srv stopped", async () => {
        aServer();
        const client = await aReadyClient();
        hear?.({ server_id: "srv-1", message: { jsonrpc: "2.0", method: NOT_ON_HOST_NOTICE, params: {} } });
        const calls = send.mock.calls.length;
        await client.didOpen("/proj/src/b.ts", "", "typescript");
        await client.didChange("/proj/src/b.ts", "x");
        expect(send.mock.calls.length).toBe(calls);
    });
});

describe("a file's address as the server is given it", () => {
    const opened = async (path: string) => {
        aServer();
        const client = await aReadyClient();
        await client.didOpen(path, "", "typescript");
        await settle();
        const did = send.mock.calls.map((c) => c[1].message as Sent).find((m) => m.method === "textDocument/didOpen");
        return did?.params?.textDocument?.uri;
    };

    it("is file:/// and the drive for a Windows drive path", async () => {
        expect(await opened("C:\\proj\\src\\a b.ts")).toBe("file:///C:/proj/src/a%20b.ts");
    });

    it("is file://server/share/ for a Windows network path, the server being the host", async () => {
        expect(await opened("\\\\fileserver\\share\\src\\a b.ts")).toBe("file://fileserver/share/src/a%20b.ts");
    });

    it("is file:// and the path for a POSIX path", async () => {
        expect(await opened("/proj/src/a.ts")).toBe("file:///proj/src/a.ts");
    });
});

describe("a document srv says is not a file", () => {
    it("is not served, and the server and the Editor go on, not shown as a crash", async () => {
        aServer((m) => (m.method === "textDocument/didOpen" ? `${NOT_A_FILE_ERROR}: srv-1` : null));
        const client = await aReadyClient();
        await client.didOpen("/proj/src/a.ts", "", "typescript");
        await settle();
        expect(client.getState().kind).toBe("ready");
    });
});
