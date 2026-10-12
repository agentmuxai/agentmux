// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { AUTH_KEY_HEADER } from "@/util/sharedconst";
import { type WebSocket, newWebSocket } from "@/util/wsutil";
import debug from "debug";
import { sprintf } from "sprintf-js";

const dlog = debug("wave:ws");

const WarnWebSocketSendSize = 1024 * 1024; // 1MB
const MaxWebSocketSendSize = 5 * 1024 * 1024; // 5MB
const reconnectHandlers: (() => void)[] = [];
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
    msgQueueTimer: ReturnType<typeof setTimeout> = null;
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
            dlog(sprintf("sleeping %ds", timeout));
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
        this.opening = false;
        this.onOpenTimeoutId = setTimeout(() => {
            this.reconnectTimes = 0;
            dlog("clear reconnect times");
        }, StableConnTime);
        for (let handler of reconnectHandlers) {
            handler();
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
        this.wsConn.send(msg);
    }

    pushMessage(data: WSCommandType) {
        // Behind a queue that is still draining, so order is kept.
        if (!this.open || this.msgQueue.length > 0) {
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
};
