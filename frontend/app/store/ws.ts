// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { AUTH_KEY_HEADER } from "@/util/sharedconst";
import { InputHold, isInputFrame } from "./ws-input-hold";
import { type WebSocket, newWebSocket } from "@/util/wsutil";
import { debugLog } from "@/util/debuglog";
import { createSlowHandlerWatch } from "./ws-slow-handler";
import { createHandlerCostTally } from "./ws-handler-cost";

const dlog = debugLog("wave:ws");

const WarnWebSocketSendSize = 1024 * 1024; // 1MB
const MaxWebSocketSendSize = 5 * 1024 * 1024; // 5MB
const watchSlowHandler = createSlowHandlerWatch();
const noteHandlerCost = createHandlerCostTally();
const reconnectHandlers: (() => void)[] = [];

/** True while the window's proof for the current connection is expected or in the making; its input frames wait (ws-input-hold.ts). */
let inputProofPending: () => boolean = () => false;
/** The gate lapsed on its own (the proof did not come in time): input goes to srv, which answers it. Reset on each new connection. */
let inputGateForcedOpen = false;
function setInputProofGate(pending: () => boolean): void {
    inputProofPending = pending;
}
const gateClosed = () => !inputGateForcedOpen && inputProofPending();
/** The most input frames (and payload bytes) kept after sending, to send again the ones srv refused for a lapsed proof. */
const RECENT_INPUT_MAX_FRAMES = 256;
const RECENT_INPUT_MAX_BYTES = 1 << 20;
/** A refusal this soon after the refused frames were sent again is not answered with another resend. */
const RESEND_COOLDOWN_MS = 3000;
const StableConnTime = 2000;
// Gap between queued `blockinput` chunks when the queue drains after a
// reconnect. Mirrors PASTE_CHUNK_DELAY_MS in block-input-sender.ts (not
// imported: that module imports this one).
const QueuedInputPaceMs = 5;

function addWSReconnectHandler(handler: () => void) {
    reconnectHandlers.push(handler);
}

type WSEventCallback = (arg0: WSEventType) => void;

type WSAuthOpts = {
    authKey: string;
};

class WSControl {
    wsConn: WebSocket;
    open: boolean;
    opening: boolean = false;
    reconnectTimes: number = 0;
    msgQueue: any[] = [];
    heldInput = new InputHold<any>(() => {
        inputGateForcedOpen = true;
        this.releaseHeldInput(false);
    });
    /** The input frames sent last, oldest first, and how many of the newest srv refused as from an unproven connection. */
    recentInput: any[] = [];
    recentInputBytes = 0;
    refusedInput = 0;
    lastResendAt = 0;
    msgQueueTimer: ReturnType<typeof setTimeout> = null;
    inReconnectHandlers: boolean = false;
    tabId: string;
    messageCallback: WSEventCallback;
    watchSessionId: string = null;
    watchScreenId: string = null;
    wsLog: string[] = [];
    baseHostPort: string;
    lastReconnectTime: number = 0;
    authOpts: WSAuthOpts;
    noReconnect: boolean = false;
    onOpenTimeoutId: NodeJS.Timeout = null;

    constructor(
        baseHostPort: string,
        tabId: string,
        messageCallback: WSEventCallback,
        wsAuthOpts?: WSAuthOpts
    ) {
        this.baseHostPort = baseHostPort;
        this.messageCallback = messageCallback;
        this.tabId = tabId;
        this.open = false;
        this.authOpts = wsAuthOpts;
        setInterval(this.sendPing.bind(this), 5000);
    }

    shutdown() {
        this.noReconnect = true;
        this.wsConn.close();
    }

    /// Update the base endpoint and reconnect.  Called after a backend restart
    /// when the port may have changed.
    changeEndpoint(newBaseHostPort: string) {
        this.baseHostPort = newBaseHostPort;
        this.reconnectTimes = 0;
        this.noReconnect = false;
        if (this.wsConn) {
            // Detach the onclose handler before closing — the socket may already be
            // CLOSED (dead sidecar, gave up reconnecting) so calling .close() won't
            // fire onclose again, and even if it does, the open/opening guard would
            // skip reconnect(). Go straight to connectNow() instead.
            this.wsConn.onclose = null;
            this.wsConn.close();
            this.wsConn = null;
        }
        this.open = false;
        this.opening = false;
        this.connectNow("changeEndpoint");
    }

    connectNow(desc: string) {
        if (this.open || this.noReconnect) {
            return;
        }
        this.lastReconnectTime = Date.now();
        dlog("try reconnect:", desc);
        this.opening = true;
        this.wsConn = newWebSocket(
            this.baseHostPort + "/ws?tabid=" + this.tabId,
            this.authOpts
                ? {
                      [AUTH_KEY_HEADER]: this.authOpts.authKey,
                  }
                : null
        );
        this.wsConn.onopen = (e: Event) => {
            this.onopen(e);
        };
        this.wsConn.onmessage = (e: MessageEvent) => {
            this.onmessage(e);
        };
        this.wsConn.onclose = (e: CloseEvent) => {
            this.onclose(e);
        };
        // turns out onerror is not necessary (onclose always follows onerror)
        // this.wsConn.onerror = this.onerror;
    }

    reconnect(forceClose?: boolean) {
        if (this.noReconnect) {
            return;
        }
        if (this.open) {
            if (forceClose) {
                this.wsConn.close(); // this will force a reconnect
            }
            return;
        }
        this.reconnectTimes++;
        if (this.reconnectTimes > 20) {
            dlog("cannot connect, giving up");
            return;
        }
        const timeoutArr = [0, 0, 2, 5, 10, 10, 30, 60];
        let timeout = 60;
        if (this.reconnectTimes < timeoutArr.length) {
            timeout = timeoutArr[this.reconnectTimes];
        }
        if (Date.now() - this.lastReconnectTime < 500) {
            timeout = 1;
        }
        if (timeout > 0) {
            dlog(`sleeping ${timeout}s`);
        }
        setTimeout(() => {
            this.connectNow(String(this.reconnectTimes));
        }, timeout * 1000);
    }

    onclose(event: CloseEvent) {
        // console.log("close", event);
        if (this.onOpenTimeoutId) {
            clearTimeout(this.onOpenTimeoutId);
        }
        if (event.wasClean) {
            dlog("connection closed");
        } else {
            dlog("connection error/disconnected");
        }
        if (this.open || this.opening) {
            this.open = false;
            this.opening = false;
            this.reconnect();
        }
    }

    onopen(e: Event) {
        dlog("connection open");
        this.open = true;
        inputGateForcedOpen = false;
        this.opening = false;
        this.onOpenTimeoutId = setTimeout(() => {
            this.reconnectTimes = 0;
            dlog("clear reconnect times");
        }, StableConnTime);
        // What the reconnect handlers send (re-subscriptions above all) goes
        // out straight away, ahead of anything queued while disconnected, so
        // the new connection is subscribed before queued input or RPCs run.
        this.inReconnectHandlers = true;
        try {
            for (let handler of reconnectHandlers) {
                handler();
            }
        } finally {
            this.inReconnectHandlers = false;
        }
        this.runMsgQueue();
    }

    /// Send what was queued while the socket wasn't open, in order. Control
    /// messages (event subscriptions, RPCs) go out at once; after each queued
    /// `blockinput` chunk the drain waits `QueuedInputPaceMs`, the pacing a
    /// paste interrupted by a reconnect would have had. It used to send one
    /// message per 100 ms whatever it was: startup queues about 16 messages,
    /// so an RPC behind them, `GetFullConfig` in `initMux`, waited 1.3-1.8 s on
    /// launches where the socket wasn't open yet.
    runMsgQueue() {
        if (this.msgQueueTimer != null) {
            return;
        }
        while (this.open && this.msgQueue.length > 0) {
            const msg = this.msgQueue.shift();
            if (isInputFrame(msg) && gateClosed()) {
                this.heldInput.hold(msg);
                continue;
            }
            this.sendMessage(msg);
            if (msg.wscommand == "blockinput" && this.msgQueue.length > 0) {
                this.msgQueueTimer = setTimeout(() => {
                    this.msgQueueTimer = null;
                    this.runMsgQueue();
                }, QueuedInputPaceMs);
                return;
            }
        }
    }

    onmessage(event: MessageEvent) {
        const handlingStart = performance.now();
        let eventData = null;
        if (event.data != null) {
            eventData = JSON.parse(event.data);
        }
        if (eventData == null) {
            return;
        }
        if (eventData.type == "ping") {
            this.wsConn.send(JSON.stringify({ type: "pong", stime: Date.now() }));
            return;
        }
        if (eventData.type == "pong") {
            // nothing
            return;
        }
        if (this.messageCallback) {
            try {
                this.messageCallback(eventData);
            } catch (e) {
                console.log("[error] messageCallback", e);
            }
        }
        // A message handled in 200 ms or more is named in the log (ws-slow-handler.ts).
        watchSlowHandler(handlingStart, eventData, typeof event.data === "string" ? event.data.length : 0);
        // Every message's time, by kind, one log line a minute when the page spent real time here (ws-handler-cost.ts).
        noteHandlerCost(handlingStart, eventData);
    }

    sendPing() {
        if (!this.open) {
            return;
        }
        this.wsConn.send(JSON.stringify({ type: "ping", stime: Date.now() }));
    }

    sendMessage(data: WSCommandType) {
        if (!this.open) {
            return;
        }
        const msg = JSON.stringify(data);
        const byteSize = new Blob([msg]).size;
        if (byteSize > MaxWebSocketSendSize) {
            console.log("ws message too large", byteSize, data.wscommand, msg.substring(0, 100));
            return;
        }
        if (byteSize > WarnWebSocketSendSize) {
            console.log("ws message large", byteSize, data.wscommand, msg.substring(0, 100));
        }
        if (isInputFrame(data)) this.keepRecentInput(data);
        this.wsConn.send(msg);
    }

    private keepRecentInput(frame: any) {
        this.recentInput.push(frame);
        this.recentInputBytes += (frame.inputdata64?.length ?? 0) + 64;
        while (this.recentInput.length > RECENT_INPUT_MAX_FRAMES || this.recentInputBytes > RECENT_INPUT_MAX_BYTES) {
            const old = this.recentInput.shift();
            this.recentInputBytes -= (old.inputdata64?.length ?? 0) + 64;
        }
    }

    /// srv refused an input frame because this connection's proof had lapsed. True when it is to be sent again once the
    /// proof is made anew; false for a refusal that follows a resend at once (the proof did not help), which is final.
    noteInputRefused(now: number = Date.now()): boolean {
        if (now - this.lastResendAt < RESEND_COOLDOWN_MS) return false;
        this.refusedInput++;
        return true;
    }

    /// The proof could not be made anew: the refused frames are not sent again.
    dropRefusedInput() {
        this.refusedInput = 0;
    }

    /// The refused frames, oldest first. srv refuses every frame from the moment the proof lapses, so they are the newest
    /// ones sent; the proof is made anew over the same ordered connection, so every refusal has arrived by then.
    private takeRefusedInput(now: number = Date.now()): any[] {
        const n = Math.min(this.refusedInput, this.recentInput.length);
        this.refusedInput = 0;
        if (n === 0) return [];
        const frames = this.recentInput.splice(this.recentInput.length - n, n);
        this.recentInputBytes = this.recentInput.reduce((sum, f) => sum + (f.inputdata64?.length ?? 0) + 64, 0);
        this.lastResendAt = now;
        return frames;
    }

    /// The window's proof has settled (or the wait ran out): send the frames srv refused for a lapsed proof, then what was
    /// held, in order, ahead of what queued since.
    releaseHeldInput(proven: boolean) {
        // Without a proof the refused frames are not sent again: srv would refuse them again.
        if (!proven) this.dropRefusedInput();
        const frames = [...(proven ? this.takeRefusedInput() : []), ...this.heldInput.take()];
        if (frames.length === 0) return;
        this.msgQueue.unshift(...frames);
        this.runMsgQueue();
    }

    pushMessage(data: WSCommandType) {
        // Input waits for the connection's proof, behind what is already held so order is kept. Not while older frames
        // are still queued (a paste draining across a reconnect): it goes behind them, and the drain holds each in turn.
        if (isInputFrame(data) && this.msgQueue.length === 0 && (this.heldInput.size > 0 || (this.open && gateClosed()))) {
            this.heldInput.hold(data);
            return;
        }
        // Behind a queue that is still draining, so order is kept; except
        // from the reconnect handlers, which go first (see onopen).
        if (!this.open || (this.msgQueue.length > 0 && !this.inReconnectHandlers)) {
            this.msgQueue.push(data);
            return;
        }
        this.sendMessage(data);
    }
}

let globalWS: WSControl;

/// Reconnect the global WS to a new endpoint.
/// Called when `backend-ready` fires after a user-initiated backend restart.
function reconnectWS(newBaseHostPort: string) {
    globalWS?.changeEndpoint(newBaseHostPort);
}

function initGlobalWS(
    baseHostPort: string,
    tabId: string,
    messageCallback: WSEventCallback,
    wsAuthOpts?: WSAuthOpts
) {
    globalWS = new WSControl(baseHostPort, tabId, messageCallback, wsAuthOpts);
}

function sendRawRpcMessage(msg: RpcMessage) {
    const wsMsg: WSRpcCommand = { wscommand: "rpc", message: msg };
    sendWSCommand(wsMsg);
}

function sendWSCommand(cmd: WSCommandType) {
    globalWS?.pushMessage(cmd);
}

export {
    WSControl,
    addWSReconnectHandler,
    globalWS,
    initGlobalWS,
    reconnectWS,
    sendRawRpcMessage,
    sendWSCommand,
    setInputProofGate,
};
