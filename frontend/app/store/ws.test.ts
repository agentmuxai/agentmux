// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";
import { WSControl, addWSReconnectHandler } from "./ws";

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

const cmd = (fields: Record<string, unknown>) => fields as unknown as WSCommandType;

vi.mock("@/util/wsutil", () => ({
    newWebSocket: () => {
        lastSocket = new FakeSocket();
        return lastSocket;
    },
}));

describe("WSControl message queue", () => {
    afterEach(() => {
        vi.useRealTimers();
    });

    it("sends everything queued before the socket opened, in order, as soon as it opens", () => {
        vi.useFakeTimers();
        const ws = new WSControl("ws://127.0.0.1:1", "tab", () => {});
        ws.connectNow("test");
        for (let i = 0; i < 16; i++) {
            ws.pushMessage(cmd({ wscommand: "eventsub", n: i }));
        }
        ws.pushMessage(cmd({ wscommand: "rpc", command: "getfullconfig" }));
        expect(lastSocket.sent).toHaveLength(0);

        lastSocket.onopen(new Event("open"));

        // No timer has run: the whole queue went out synchronously on open.
        const sent = lastSocket.sent.map((m) => JSON.parse(m));
        expect(sent).toHaveLength(17);
        expect(sent.slice(0, 16).map((m) => m.n)).toEqual([...Array(16).keys()]);
        expect(sent[16].command).toBe("getfullconfig");
        expect(ws.msgQueue).toHaveLength(0);
    });

    it("keeps queued paste chunks paced, and keeps order with messages pushed meanwhile", () => {
        vi.useFakeTimers();
        const ws = new WSControl("ws://127.0.0.1:1", "tab", () => {});
        ws.connectNow("test");
        ws.pushMessage(cmd({ wscommand: "eventsub", tag: "sub" }));
        ws.pushMessage(cmd({ wscommand: "blockinput", tag: "in1" }));
        ws.pushMessage(cmd({ wscommand: "blockinput", tag: "in2" }));
        ws.pushMessage(cmd({ wscommand: "rpc", tag: "rpc" }));
        const tags = () => lastSocket.sent.map((m) => JSON.parse(m).tag);

        lastSocket.onopen(new Event("open"));
        expect(tags()).toEqual(["sub", "in1"]);

        // Pushed while the drain is paused: queued behind, not sent ahead.
        ws.pushMessage(cmd({ wscommand: "rpc", tag: "late" }));
        expect(tags()).toEqual(["sub", "in1"]);

        vi.advanceTimersByTime(5);
        expect(tags()).toEqual(["sub", "in1", "in2"]);
        vi.advanceTimersByTime(5);
        expect(tags()).toEqual(["sub", "in1", "in2", "rpc", "late"]);
        expect(ws.msgQueue).toHaveLength(0);

        ws.pushMessage(cmd({ wscommand: "rpc", tag: "direct" }));
        expect(tags().at(-1)).toBe("direct");
    });

    it("sends what the reconnect handlers push before the queued backlog", () => {
        const ws = new WSControl("ws://127.0.0.1:1", "tab", () => {});
        let resubscribe = false;
        addWSReconnectHandler(() => {
            if (resubscribe) {
                ws.pushMessage(cmd({ wscommand: "eventsub", tag: "resub" }));
            }
        });
        ws.connectNow("test");
        ws.pushMessage(cmd({ wscommand: "blockinput", tag: "queued-input" }));
        ws.pushMessage(cmd({ wscommand: "rpc", tag: "queued-rpc" }));
        resubscribe = true;

        lastSocket.onopen(new Event("open"));

        expect(lastSocket.sent.map((m) => JSON.parse(m).tag)).toEqual(["resub", "queued-input"]);
    });

    it("sends directly once open", () => {
        const ws = new WSControl("ws://127.0.0.1:1", "tab", () => {});
        ws.connectNow("test");
        lastSocket.onopen(new Event("open"));
        ws.pushMessage(cmd({ wscommand: "rpc", command: "x" }));
        expect(lastSocket.sent.map((m) => JSON.parse(m).command)).toEqual(["x"]);
    });
});
