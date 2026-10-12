// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";

class FakeSocket {
    sent: string[] = [];
    onopen: (e: Event) => void = null;
    onmessage: (e: MessageEvent) => void = null;
    onclose: (e: CloseEvent) => void = null;
    send(msg: string) {
        this.sent.push(msg);
    }
    close() {}
}

let lastSocket: FakeSocket = null;

vi.mock("@/util/wsutil", () => ({
    newWebSocket: () => {
        lastSocket = new FakeSocket();
        return lastSocket;
    },
}));

const { WSControl } = await import("./ws");

describe("WSControl message queue", () => {
    afterEach(() => {
        vi.useRealTimers();
    });

    it("sends everything queued before the socket opened, in order, as soon as it opens", () => {
        vi.useFakeTimers();
        const ws = new WSControl("ws://127.0.0.1:1", "tab", () => {});
        ws.connectNow("test");
        for (let i = 0; i < 16; i++) {
            ws.pushMessage({ wscommand: "eventsub", n: i } as any);
        }
        ws.pushMessage({ wscommand: "rpc", command: "getfullconfig" } as any);
        expect(lastSocket.sent).toHaveLength(0);

        lastSocket.onopen(new Event("open"));

        // No timer has run: the whole queue went out synchronously on open.
        const sent = lastSocket.sent.map((m) => JSON.parse(m));
        expect(sent).toHaveLength(17);
        expect(sent.slice(0, 16).map((m) => m.n)).toEqual([...Array(16).keys()]);
        expect(sent[16].command).toBe("getfullconfig");
        expect(ws.msgQueue).toHaveLength(0);
    });

    it("sends directly once open", () => {
        const ws = new WSControl("ws://127.0.0.1:1", "tab", () => {});
        ws.connectNow("test");
        lastSocket.onopen(new Event("open"));
        ws.pushMessage({ wscommand: "rpc", command: "x" } as any);
        expect(lastSocket.sent.map((m) => JSON.parse(m).command)).toEqual(["x"]);
    });
});
