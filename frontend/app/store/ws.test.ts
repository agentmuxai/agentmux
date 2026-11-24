// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { WSControl, addWSReconnectHandler, setInputProofGate } from "./ws";
import { notifications, setNotifications } from "./flash-notifications";
import { HOLD_MAX_FRAMES, HOLD_MAX_MS, resetInputNoticeForTests } from "./ws-input-hold";

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


describe("WSControl input waits for the window's proof", () => {
    let pending = false;
    const tagsOf = () => lastSocket.sent.map((m) => JSON.parse(m).tag).filter((t) => t !== undefined); // keepalive frames carry no tag
    const open = () => {
        const ws = new WSControl("ws://127.0.0.1:1", "tab", () => {});
        ws.connectNow("test");
        lastSocket.onopen(new Event("open"));
        return ws;
    };
    const settle = (ws: WSControl) => {
        pending = false;
        ws.releaseHeldInput(true);
    };

    beforeEach(() => {
        vi.useFakeTimers();
        pending = false;
        setInputProofGate(() => pending);
        setNotifications([]);
        resetInputNoticeForTests();
    });
    afterEach(() => {
        vi.useRealTimers();
        setInputProofGate(() => false);
    });

    it("holds typed frames while the proof is pending and sends them in order once it settles, rpc calls not waiting", () => {
        const ws = open();
        pending = true;
        ws.pushMessage(cmd({ wscommand: "blockinput", tag: "k1" }));
        ws.pushMessage(cmd({ wscommand: "rpc", tag: "rpc" }));
        ws.pushMessage(cmd({ wscommand: "blockinput", tag: "k2" }));
        ws.pushMessage(cmd({ wscommand: "setblocktermsize", tag: "size" }));
        expect(tagsOf()).toEqual(["rpc"]);
        settle(ws);
        vi.advanceTimersByTime(1000);
        expect(tagsOf()).toEqual(["rpc", "k1", "k2", "size"]);
    });

    it("sends a frame typed after the proof at once, and one typed behind a held backlog goes behind it", () => {
        const ws = open();
        ws.pushMessage(cmd({ wscommand: "blockinput", tag: "now" }));
        expect(tagsOf()).toEqual(["now"]);
        pending = true;
        ws.pushMessage(cmd({ wscommand: "blockinput", tag: "held" }));
        pending = false; // the proof settled but the release has not run yet: a new frame still goes behind the held one
        ws.pushMessage(cmd({ wscommand: "blockinput", tag: "after" }));
        expect(tagsOf()).toEqual(["now"]);
        ws.releaseHeldInput(true);
        vi.advanceTimersByTime(1000);
        expect(tagsOf()).toEqual(["now", "held", "after"]);
    });

    it("keeps what was typed while disconnected, holds it on reconnect until the proof settles, and sends each frame once, in order", () => {
        pending = true;
        const ws = new WSControl("ws://127.0.0.1:1", "tab", () => {});
        ws.connectNow("test");
        ws.pushMessage(cmd({ wscommand: "blockinput", tag: "a" }));
        ws.pushMessage(cmd({ wscommand: "blockinput", tag: "b" }));
        ws.pushMessage(cmd({ wscommand: "rpc", tag: "rpc" }));
        lastSocket.onopen(new Event("open"));
        expect(tagsOf()).toEqual(["rpc"]);
        ws.pushMessage(cmd({ wscommand: "blockinput", tag: "c" }));
        settle(ws);
        vi.advanceTimersByTime(1000);
        expect(tagsOf()).toEqual(["rpc", "a", "b", "c"]);
        expect(ws.msgQueue).toHaveLength(0);
        expect(ws.heldInput.size).toBe(0);
    });

    it("at the bound keeps what it holds, drops the newest, and tells the person", () => {
        const ws = open();
        pending = true;
        for (let i = 0; i < HOLD_MAX_FRAMES + 5; i++) ws.pushMessage(cmd({ wscommand: "blockinput", tag: i }));
        expect(ws.heldInput.size).toBe(HOLD_MAX_FRAMES);
        expect(notifications().filter((n) => n.title === "Input not sent")).toHaveLength(1);
        settle(ws);
        vi.advanceTimersByTime(60 * 60 * 1000);
        const sent = tagsOf();
        expect(sent).toEqual([...Array(HOLD_MAX_FRAMES).keys()]);
    });

    it("sends what it holds when the proof does not come in time, so srv answers it and the person is told", () => {
        const ws = open();
        pending = true;
        ws.pushMessage(cmd({ wscommand: "blockinput", tag: "k" }));
        vi.advanceTimersByTime(HOLD_MAX_MS + 10);
        expect(tagsOf()).toEqual(["k"]);
        // The gate stays open for the rest of this connection, so later frames are not held for a proof that is not coming.
        ws.pushMessage(cmd({ wscommand: "blockinput", tag: "k2" }));
        expect(tagsOf()).toEqual(["k", "k2"]);
    });

    it("sends again, once the proof is made anew, the frames srv refused for a lapsed proof, ahead of what was held meanwhile", () => {
        const ws = open();
        for (const tag of ["k1", "k2", "k3"]) ws.pushMessage(cmd({ wscommand: "blockinput", tag }));
        // srv refused the last two (the proof lapsed between the first and the second); proving again begins, so input is held
        expect(ws.noteInputRefused()).toBe(true);
        expect(ws.noteInputRefused()).toBe(true);
        pending = true;
        ws.pushMessage(cmd({ wscommand: "blockinput", tag: "k4" }));
        settle(ws);
        vi.advanceTimersByTime(1000);
        expect(tagsOf()).toEqual(["k1", "k2", "k3", "k2", "k3", "k4"]);
    });

    it("does not send refused frames again when the proof could not be made anew, nor answer a refusal right after a resend with another", () => {
        const ws = open();
        ws.pushMessage(cmd({ wscommand: "blockinput", tag: "k1" }));
        expect(ws.noteInputRefused()).toBe(true);
        ws.releaseHeldInput(false);
        vi.advanceTimersByTime(1000);
        expect(tagsOf()).toEqual(["k1"]);

        expect(ws.noteInputRefused()).toBe(true);
        ws.releaseHeldInput(true);
        vi.advanceTimersByTime(1000);
        expect(tagsOf()).toEqual(["k1", "k1"]);
        expect(ws.noteInputRefused()).toBe(false); // refused again at once: the proof did not help, so it is final
    });

    it("keeps order when a reconnect interrupts a paste that is still draining", () => {
        const ws = new WSControl("ws://127.0.0.1:1", "tab", () => {});
        ws.connectNow("test");
        for (const tag of ["A", "B", "C"]) ws.pushMessage(cmd({ wscommand: "blockinput", tag }));
        lastSocket.onopen(new Event("open"));
        expect(tagsOf()).toEqual(["A"]); // B and C wait for the pacing timer
        pending = true; // the connection dropped and came back: its proof is pending again
        ws.pushMessage(cmd({ wscommand: "blockinput", tag: "D" }));
        vi.advanceTimersByTime(100);
        settle(ws);
        vi.advanceTimersByTime(1000);
        expect(tagsOf()).toEqual(["A", "B", "C", "D"]);
    });
});
